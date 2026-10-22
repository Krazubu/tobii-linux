//! Client requests: what the device reported about itself (identity,
//! geometry, stream catalogue and the streams' columns, hardware
//! configuration, fault and warning lists), its display area, states and
//! clock.
//! Calibration requests are handed to [`crate::calibration`], the device
//! name to [`crate::name`], pause and resume to [`crate::pause`], and the
//! starts and stops of the tracker's catalogue streams, the custom stream
//! and image collection, to [`crate::catalogue`].
//!
//! Identity and geometry come from the facts collected during the engine's
//! init, so answering them costs no USB traffic; a request made while the
//! engine is still starting waits for them. A stream's columns come from
//! there too: the init asks the tracker for them (command 1210), where the
//! DLL asks at every call, and no request sends one.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tobii_ipc::deadline;
use tobii_ipc::geometry::{DisplayArea, display_area_basic};
use tobii_ipc::request::{
    self, Request, Timesync, decode_display_area, encode_device_info, encode_display_area,
    encode_geometry_mounting, encode_hardware_configuration, encode_stream_columns,
    encode_stream_types, encode_timesync, encode_track_box, encode_u32, kind, state, status,
};
use tobii_proto::facts::{
    DEFAULT_DISPLAY_ID, DeviceFacts, STATUS_FAULTS, STATUS_POWER_SAVE, STATUS_REMOTE_WAKE,
    STATUS_WARNINGS, display_area_set_payload,
};
use tobii_proto::protocol::cmd;
use tracing::{info, warn};

use crate::daemon::{State, is_tracker_present, lock_state, look_for_tracker};

/// A reply: Stream Engine status code and kind-specific payload, and what
/// the request leaves to run once the reply is queued, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reply {
    pub(crate) status: u8,
    pub(crate) payload: Vec<u8>,
    /// Run once the reply is queued, on a thread of its own, so that the
    /// client's next request does not wait for it (see
    /// `daemon::run_requests`).
    pub(crate) then: Option<FollowUp>,
}

impl Reply {
    pub(crate) fn ok(payload: Vec<u8>) -> Self {
        Self {
            status: status::OK,
            payload,
            then: None,
        }
    }

    pub(crate) fn err(status: u8) -> Self {
        Self {
            status,
            payload: Vec::new(),
            then: None,
        }
    }
}

/// What a request leaves to run once its reply is queued ([`Reply::then`]):
/// work its client is not to wait for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FollowUp {
    /// Start catalogue stream `stream`, which a restore had `client` hold
    /// when nobody held it (see [`crate::catalogue::start_restored`]).
    StartRestored {
        /// The client that restored its claim.
        client: u64,
        /// The device stream id.
        stream: u32,
    },
}

impl FollowUp {
    /// Run it, on the calling thread.
    pub(crate) fn run(self, state: &Mutex<State>) {
        match self {
            Self::StartRestored { client, stream } => {
                crate::catalogue::start_restored(state, client, stream);
            }
        }
    }
}

impl From<Result<Vec<u8>, u8>> for Reply {
    fn from(r: Result<Vec<u8>, u8>) -> Self {
        r.map_or_else(Self::err, Self::ok)
    }
}

/// How long a request waits for the device's init to report its facts.
const FACTS_TIMEOUT: Duration = deadline::FACTS;
/// How long the device may take to acknowledge a display area.
const DISPLAY_AREA_TIMEOUT: Duration = deadline::DISPLAY_AREA;
/// Host-side latency bound between the device stamping a gaze frame and the
/// daemon reading it: the lower edge of the TIMESYNC window.
const FRAME_LATENCY_US: i64 = 30_000;
/// How long a TIMESYNC waits for a gaze frame newer than the request (see
/// [`deadline::TIMESYNC`]).
#[cfg(not(test))]
const TIMESYNC_TIMEOUT: Duration = deadline::TIMESYNC;
#[cfg(test)]
const TIMESYNC_TIMEOUT: Duration = Duration::from_millis(300);
/// How often a TIMESYNC looks for that frame (the tracker sends ~33 a second).
const TIMESYNC_POLL: Duration = Duration::from_millis(5);
/// How long a TIMESYNC lets no engine run before it looks for the tracker
/// on the bus: the watchdog restarts an engine lost while the tracker is
/// still there, and a re-plug or a re-enumeration takes the tracker off the
/// bus for a moment. After engines that ended before the tracker was ready
/// (it refuses to be opened, say) the watchdog backs off for up to a minute
/// (see [`crate::restart`]), so that a TIMESYNC may wait out its whole
/// deadline and end `TIMED_OUT`.
#[cfg(not(test))]
const ENGINE_LOST_GRACE: Duration = Duration::from_secs(1);
#[cfg(test)]
const ENGINE_LOST_GRACE: Duration = Duration::from_millis(100);

/// Answer one request from `client`.
pub(crate) fn handle(state: &Mutex<State>, client: u64, req: &Request<'_>) -> Reply {
    match req.kind {
        kind::DEVICE_INFO => facts(state, client, |f| {
            Some(encode_device_info(&f.device_info()))
        }),
        kind::TRACK_BOX => facts(state, client, |f| {
            f.track_box.as_ref().map(encode_track_box)
        }),
        kind::DISPLAY_AREA_GET => facts(state, client, |f| {
            f.display_area.as_ref().map(encode_display_area)
        }),
        kind::GEOMETRY_MOUNTING => facts(state, client, |f| {
            f.mounting.as_ref().map(encode_geometry_mounting)
        }),
        // From the init's 1200, not a new one per call as the DLL sends: the
        // catalogue is the firmware's.
        kind::STREAM_TYPES => facts(state, client, |f| {
            (!f.streams.is_empty()).then(|| encode_stream_types(&f.streams))
        }),
        // From the init's 1210s, not a new one per call as the DLL sends
        // (tracker_enumerate_stream_type_columns, 0x18019c9a0): the columns
        // are the firmware's. Without an answer to give (a stream the init
        // did not ask about, or one the tracker refused), NOT_SUPPORTED.
        kind::STREAM_COLUMNS => match request::decode_u32(req.payload) {
            Some(id) => facts(state, client, |f| {
                f.stream_columns(id).map(encode_stream_columns)
            }),
            None => Reply::err(status::INVALID_PARAMETER),
        },
        // Provisional, and NOT_SUPPORTED so far: the ET5 answers the init's
        // 2120 with an empty payload on Linux.
        kind::HARDWARE_CONFIGURATION => facts(state, client, |f| {
            f.hardware.as_ref().map(encode_hardware_configuration)
        }),
        kind::DISPLAY_AREA_SET => match decode_display_area(req.payload) {
            Some(area) if is_finite(&area) => set_display_area(state, client, area),
            _ => Reply::err(status::INVALID_PARAMETER),
        },
        kind::STATE => match request::decode_u32(req.payload) {
            Some(state::CALIBRATION_ID) => {
                let id = lock_state(state).calibration.id;
                id.map_or_else(
                    || Reply::err(status::NOT_AVAILABLE),
                    |id| Reply::ok(encode_u32(id)),
                )
            }
            Some(state::CALIBRATION_ACTIVE) => {
                let active = lock_state(state).calibration.is_active();
                Reply::ok(vec![u8::from(active)])
            }
            Some(state::DEVICE_PAUSED) => Reply::ok(vec![u8::from(lock_state(state).paused)]),
            // From the last init's 1490 and the 3200/3210 since, as the DLL
            // answers from the cache its create and reconnect fill and those
            // notifications update: no 1490 per call.
            Some(id @ (state::FAULT | state::WARNING)) => {
                let index = if id == state::FAULT {
                    STATUS_FAULTS
                } else {
                    STATUS_WARNINGS
                };
                facts(state, client, |f| {
                    f.status_string(index).map(|s| s.as_bytes().to_vec())
                })
            }
            // From the last init's 1490 and, for power save, the 3020/3030
            // since, as the DLL's getters read a byte its create and reconnect
            // fill and only those notifications rewrite (see the constants):
            // no command per call. A value that is not a boolean is no value
            // at all to the DLL, whose 1490 reader stores neither it nor its
            // "present" flag then.
            Some(id @ (state::POWER_SAVE_ACTIVE | state::REMOTE_WAKE_ACTIVE)) => {
                let index = if id == state::POWER_SAVE_ACTIVE {
                    STATUS_POWER_SAVE
                } else {
                    STATUS_REMOTE_WAKE
                };
                facts(state, client, |f| match f.status_string(index) {
                    Some("true") => Some(vec![1]),
                    Some("false") => Some(vec![0]),
                    _ => None,
                })
            }
            Some(_) => Reply::err(status::NOT_SUPPORTED),
            None => Reply::err(status::INVALID_PARAMETER),
        },
        kind::TIMESYNC => timesync(state, client),
        kind::DEVICE_NAME_GET => crate::name::get(state, client),
        kind::DEVICE_NAME_SET => crate::name::set(state, req.payload),
        kind::DEVICE_PAUSE => crate::pause::handle(state, client, req.payload),
        kind::POWER_SAVE => crate::power_save::handle(state, client, req.payload),
        kind::EXTENSIONS => crate::extensions::handle(state, client, req.payload),
        kind::CATALOGUE_STREAM_START => crate::catalogue::start(state, client, req.payload),
        kind::CATALOGUE_STREAM_STOP => crate::catalogue::stop(state, client, req.payload),
        k if (kind::CALIBRATION_START..=kind::CALIBRATION_CLEAR).contains(&k) => {
            crate::calibration::handle(state, client, k, req.payload)
        }
        _ => Reply::err(status::NOT_SUPPORTED),
    }
}

fn is_finite(area: &DisplayArea) -> bool {
    [area.top_left_mm, area.top_right_mm, area.bottom_left_mm]
        .iter()
        .flatten()
        .all(|v| v.is_finite())
}

/// Wait for the device's facts (starting the engine if needed) and answer
/// from them; `NOT_SUPPORTED` when the device did not report that fact.
pub(crate) fn facts(
    state: &Mutex<State>,
    client: u64,
    answer: impl Fn(&DeviceFacts) -> Option<Vec<u8>>,
) -> Reply {
    let on_bus = look_for_tracker(state);
    let _ = lock_state(state).device_for(client, on_bus);
    match wait_for_facts(state) {
        Ok(facts) => answer(&facts).map_or_else(|| Reply::err(status::NOT_SUPPORTED), Reply::ok),
        Err(code) => Reply::err(code),
    }
}

/// The facts of the device's last init (kept across engines), waited for
/// up to [`FACTS_TIMEOUT`] while there are none yet: `TIMED_OUT` then. It
/// starts no engine; its caller fetches the device first (see
/// [`State::device_for`]).
pub(crate) fn wait_for_facts(state: &Mutex<State>) -> Result<Arc<DeviceFacts>, u8> {
    let started = Instant::now();
    loop {
        if let Some(facts) = lock_state(state).facts.clone() {
            return Ok(facts);
        }
        if started.elapsed() > FACTS_TIMEOUT {
            return Err(status::TIMED_OUT);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// A device/host clock pair from the first gaze frame seen after the
/// request, so that it is fresh and belongs to the running device session.
/// Frames are counted rather than their host times compared with the
/// request's: one read just before the request and handed over after it is
/// still new to it. Starts the engine if needed and, as the facts do,
/// keeps it up while `client` stays connected. `NOT_AVAILABLE` while the
/// device is paused, since it sends none. `CONNECTION_FAILED` when no
/// tracker can send one: at once when there is none on the bus to start an
/// engine for, and, when the engine is lost during the wait, once none has
/// run for [`ENGINE_LOST_GRACE`] and the tracker is off the bus. `TIMED_OUT`
/// if no such frame arrives in time otherwise.
fn timesync(state: &Mutex<State>, client: u64) -> Reply {
    let started = Instant::now();
    let on_bus = look_for_tracker(state);
    let asked = {
        let mut st = lock_state(state);
        let _ = st.device_for(client, on_bus);
        // Only a tracker on the bus gets an engine.
        if !st.is_engine_running() {
            return Reply::err(status::CONNECTION_FAILED);
        }
        st.gaze_frames
    };
    // Since when no engine has run during the wait.
    let mut lost_since: Option<Instant> = None;
    loop {
        // Copied out, so that no guard is held across the sleep.
        let (frames, clock, paused, running) = {
            let st = lock_state(state);
            (st.gaze_frames, st.clock, st.paused, st.is_engine_running())
        };
        if paused {
            return Reply::err(status::NOT_AVAILABLE);
        }
        // A stop clears `clock` after the count moved: wait for the next one.
        if frames != asked
            && let Some((device_us, host_rx_us)) = clock
        {
            return Reply::ok(encode_timesync(&Timesync {
                host_start_us: host_rx_us - FRAME_LATENCY_US,
                device_us,
                host_end_us: host_rx_us,
            }));
        }
        if running {
            lost_since = None;
        } else if lost_since.get_or_insert_with(Instant::now).elapsed() >= ENGINE_LOST_GRACE {
            // The watchdog restarts an engine only for a tracker on the
            // bus, and only once its backoff allows; the request does not
            // start one itself. The bus is scanned without the state lock.
            if !is_tracker_present(state) {
                return Reply::err(status::CONNECTION_FAILED);
            }
            lost_since = None;
        }
        if started.elapsed() > TIMESYNC_TIMEOUT {
            return Reply::err(status::TIMED_OUT);
        }
        thread::sleep(TIMESYNC_POLL);
    }
}

/// Write a display area to the device and, once it accepts it, keep it
/// across re-inits and sessions. Refused while another client calibrates
/// (its calibration is being made on the current area). During the caller's
/// own session it is saved only with the calibration computed on it, and
/// what it replaces is remembered, so that a session ending without a
/// calibration can put it back (see [`crate::calibration`]). One set inside
/// a session that ends before the device answers is neither kept nor saved
/// (`CALIBRATION_NOT_STARTED`): the end put back the area it replaced, which
/// goes to the device again after this one.
fn set_display_area(state: &Mutex<State>, client: u64, area: DisplayArea) -> Reply {
    let on_bus = look_for_tracker(state);
    let (device, display_id, in_session) = {
        let mut st = lock_state(state);
        if let Err(code) = st.calibration.display_access(client) {
            return Reply::err(code);
        }
        let device = st.device_for(client, on_bus);
        // Fetching the device may have dropped a dead engine, which took
        // the calibration session along, whether or not another started:
        // look again.
        if let Err(code) = st.calibration.display_access(client) {
            return Reply::err(code);
        }
        let Some(device) = device else {
            return Reply::err(status::CONNECTION_FAILED);
        };
        // A client's area supersedes TOBII_DISPLAY_MM for this run: do not
        // let the variable's write, queued at the next device init, undo it.
        st.display_request = None;
        let before = crate::calibration::DisplayBefore {
            device: st.facts.as_ref().and_then(|f| f.display_area),
            configured: st.display_override,
        };
        st.calibration.note_display_before(client, before);
        let display_id = st
            .facts
            .as_ref()
            .and_then(|f| f.display_id)
            .unwrap_or(DEFAULT_DISPLAY_ID);
        let in_session = st.calibration.owner() == Some(client);
        (device, display_id, in_session)
    };
    let result = crate::device::run(
        device.as_ref(),
        cmd::DISPLAY_AREA_SET,
        display_area_set_payload(&area, display_id),
        DISPLAY_AREA_TIMEOUT,
    );
    if result.is_ok() {
        let file = {
            let mut st = lock_state(state);
            // The session ended while the device took the area (the device
            // re-initialised, or the engine was lost), and its end put back
            // the area this one replaced: nothing would put this one back.
            if in_session && st.calibration.owner() != Some(client) {
                let back = st.facts.as_ref().and_then(|f| f.display_area);
                drop(st);
                warn!(
                    client,
                    "the calibration session ended as the display area was set; not kept"
                );
                // The end's own write may have reached the device ahead of
                // this one's: the area it put back goes once more.
                if let Some(back) = back {
                    put_area_back_on(device.as_ref(), &back, display_id);
                }
                return Reply::err(status::CALIBRATION_NOT_STARTED);
            }
            let save_now = st.calibration.note_display_set(client, area);
            st.display_override = Some(area);
            if let Some(engine) = st.engine.as_ref() {
                engine.set_display_area_override(Some(area));
            }
            if let Some(facts) = st.facts.as_ref() {
                let mut facts = (**facts).clone();
                facts.display_area = Some(area);
                st.facts = Some(Arc::new(facts));
            }
            st.display_file.clone().filter(|_| save_now)
        };
        // Kept across sessions, as the Stream Engine does.
        if let Some(path) = file {
            save_display_area(&path, &area);
        }
    }
    result.map(|_| Vec::new()).into()
}

/// Save `area` at `path`. The device has it already, so failing only costs
/// it at the next start. The same area again is not written (it would push
/// the rollback copy out).
pub(crate) fn save_display_area(path: &std::path::Path, area: &DisplayArea) {
    if crate::display::load(path).ok().flatten() == Some(*area) {
        return;
    }
    match crate::display::save(path, area) {
        Ok(()) => info!(
            path = %path.display(),
            width_mm = width_mm(area),
            "display area saved"
        ),
        Err(e) => warn!(path = %path.display(), error = %e, "could not save the display area"),
    }
}

/// Put the display area back as it was before a calibration session that
/// did not commit changed it: for later inits, then on the device of the
/// engine running then, if one runs. Both under one lock, so an engine
/// started in place of a lost one gets the area either from its init or
/// from this write. The saved file never had the session's area.
pub(crate) fn put_display_back(state: &Mutex<State>, before: &crate::calibration::DisplayBefore) {
    let (device, display_id) = {
        let mut st = lock_state(state);
        put_display_configuration_back(&mut st, before);
        let display_id = st
            .facts
            .as_ref()
            .and_then(|f| f.display_id)
            .unwrap_or(DEFAULT_DISPLAY_ID);
        (st.commands(), display_id)
    };
    if let (Some(device), Some(area)) = (device, before.device) {
        put_area_back_on(device.as_ref(), &area, display_id);
    }
}

/// Write `area`, one that was put back, to `device`.
fn put_area_back_on(
    device: &dyn crate::device::DeviceCommands,
    area: &DisplayArea,
    display_id: u32,
) {
    if let Err(code) = crate::device::run(
        device,
        cmd::DISPLAY_AREA_SET,
        display_area_set_payload(area, display_id),
        DISPLAY_AREA_TIMEOUT,
    ) {
        warn!(
            status = code,
            "could not put the display area back on the device"
        );
    }
}

/// [`put_display_back`] for the fan-out pump, which must not wait on the
/// device: the write runs on its own thread, as [`apply_display_request`]'s
/// does.
pub(crate) fn put_display_back_detached(
    st: &mut State,
    before: &crate::calibration::DisplayBefore,
) {
    if let (Some(device), Some(area)) = (st.commands(), before.device) {
        let display_id = st
            .facts
            .as_ref()
            .and_then(|f| f.display_id)
            .unwrap_or(DEFAULT_DISPLAY_ID);
        thread::spawn(move || put_area_back_on(device.as_ref(), &area, display_id));
    }
    put_display_configuration_back(st, before);
}

/// The configuration half of [`put_display_back`]: what later inits write.
pub(crate) fn put_display_configuration_back(
    st: &mut State,
    before: &crate::calibration::DisplayBefore,
) {
    st.display_override = before.configured;
    if let Some(engine) = st.engine.as_ref() {
        engine.set_display_area_override(before.configured);
    }
    if let Some(facts) = st.facts.as_ref() {
        let mut facts = (**facts).clone();
        facts.display_area = before.device;
        st.facts = Some(Arc::new(facts));
    }
    info!("display area put back: the calibration session did not commit");
}

/// Width of a display area, for the log.
fn width_mm(area: &DisplayArea) -> f64 {
    (0..3)
        .map(|i| (area.top_right_mm[i] - area.top_left_mm[i]).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// At start: configure the saved display area, so the engine's first init
/// already writes it. It wins over `TOBII_DISPLAY_MM`: it was set on the
/// device later (the calibration saved with it was made on it), and the
/// variable only fills in while nothing is saved.
pub(crate) fn restore_saved_display_area(st: &mut State) {
    let Some(path) = st.display_file.clone() else {
        return;
    };
    match crate::display::load(&path) {
        Ok(Some(area)) => {
            info!(
                path = %path.display(),
                width_mm = width_mm(&area),
                "display area: using the saved one"
            );
            if st.display_request.is_some() {
                info!(
                    path = %path.display(),
                    "TOBII_DISPLAY_MM is ignored while a display area is saved; delete the file to use it"
                );
            }
            st.display_override = Some(area);
        }
        Ok(None) => {}
        Err(e) => warn!(path = %path.display(), error = %e, "ignoring the saved display area"),
    }
}

/// A monitor size from `TOBII_DISPLAY_MM=<width>x<height>[+<offset_x>]` (mm):
/// the display area is computed from it and the tracker's mounting once the
/// device reports the mounting, replacing the capture author's monitor that
/// the init replay would otherwise leave configured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DisplaySize {
    width_mm: f64,
    height_mm: f64,
    offset_x_mm: f64,
}

impl DisplaySize {
    pub(crate) fn from_env() -> Option<Self> {
        let value = std::env::var("TOBII_DISPLAY_MM").ok()?;
        let parsed = Self::parse(&value);
        if parsed.is_none() {
            warn!(
                value,
                "TOBII_DISPLAY_MM: expected <width>x<height>[+<offset_x>] in mm"
            );
        }
        parsed
    }

    fn parse(s: &str) -> Option<Self> {
        let (size, offset) = s.split_once('+').map_or((s, "0"), |(a, b)| (a, b));
        let (w, h) = size.split_once(['x', 'X'])?;
        let parsed = Self {
            width_mm: w.trim().parse().ok()?,
            height_mm: h.trim().parse().ok()?,
            offset_x_mm: offset.trim().parse().ok()?,
        };
        (parsed.width_mm > 0.0 && parsed.height_mm > 0.0 && parsed.offset_x_mm.is_finite())
            .then_some(parsed)
    }
}

/// After a device init: if `TOBII_DISPLAY_MM` is set and no client has set a
/// display area, compute one from the reported mounting and write it. The
/// write runs on its own thread (the caller is the fan-out pump); later
/// re-inits pick the area up from the override.
pub(crate) fn apply_display_request(st: &mut State) {
    let (Some(size), None) = (st.display_request, st.display_override) else {
        return;
    };
    let Some(mounting) = st.facts.as_ref().and_then(|f| f.mounting) else {
        return;
    };
    let area = display_area_basic(size.width_mm, size.height_mm, size.offset_x_mm, &mounting);
    st.display_override = Some(area);
    if let Some(engine) = st.engine.as_ref() {
        engine.set_display_area_override(Some(area));
        let commands = engine.commands();
        let payload = display_area_set_payload(&area, DEFAULT_DISPLAY_ID);
        info!(
            width_mm = size.width_mm,
            height_mm = size.height_mm,
            "configuring the display area"
        );
        thread::spawn(move || {
            let device: &dyn crate::device::DeviceCommands = &commands;
            if let Err(status) =
                crate::device::run(device, cmd::DISPLAY_AREA_SET, payload, DISPLAY_AREA_TIMEOUT)
            {
                warn!(status, "could not set the display area");
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
    use tobii_ipc::host_clock_us;

    #[test]
    #[allow(clippy::float_cmp)] // reason: parsed literals
    fn parses_display_sizes() {
        let s = DisplaySize::parse("597x336").expect("size");
        assert_eq!(
            (s.width_mm, s.height_mm, s.offset_x_mm),
            (597.0, 336.0, 0.0)
        );
        let s = DisplaySize::parse(" 633.6 X 334.3 + 1.0 ").expect("size");
        assert_eq!(
            (s.width_mm, s.height_mm, s.offset_x_mm),
            (633.6, 334.3, 1.0)
        );
        assert_eq!(DisplaySize::parse("597"), None);
        assert_eq!(DisplaySize::parse("-1x2"), None);
    }

    #[test]
    fn state_requests_answer_from_the_daemon() {
        let state = Mutex::new(crate::daemon::tests::state_with_client(1));
        lock_state(&state).calibration.id = Some(1_904_654_973);
        let ask = |id| {
            handle(
                &state,
                1,
                &Request {
                    id: 9,
                    kind: kind::STATE,
                    payload: &encode_u32(id),
                },
            )
        };
        assert_eq!(
            ask(state::CALIBRATION_ID),
            Reply::ok(encode_u32(1_904_654_973))
        );
        assert_eq!(ask(state::CALIBRATION_ACTIVE), Reply::ok(vec![0]));
        assert_eq!(ask(state::DEVICE_PAUSED), Reply::ok(vec![0]));
        lock_state(&state).paused = true;
        assert_eq!(ask(state::DEVICE_PAUSED), Reply::ok(vec![1]));
        assert_eq!(ask(3), Reply::err(status::NOT_SUPPORTED));
    }

    #[test]
    fn stream_types_answer_from_the_facts() {
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(Arc::new(Answering(1)));
        let state = Mutex::new(st);
        let req = Request {
            id: 1,
            kind: kind::STREAM_TYPES,
            payload: &[],
        };
        let streams = vec![
            request::StreamType {
                id: 0x500,
                name: "gaze".into(),
                ..request::StreamType::default()
            },
            request::StreamType {
                id: 0x508,
                name: "image_collection".into(),
                text: String::new(),
                value: 1000,
            },
        ];
        lock_state(&state).facts = Some(Arc::new(DeviceFacts {
            streams: streams.clone(),
            ..DeviceFacts::default()
        }));

        let reply = handle(&state, 1, &req);

        assert_eq!(reply, Reply::ok(encode_stream_types(&streams)));
        assert!(lock_state(&state).clients[0].holds_device);

        // An init that reported no catalogue.
        lock_state(&state).facts = Some(Arc::new(DeviceFacts::default()));
        assert_eq!(handle(&state, 1, &req), Reply::err(status::NOT_SUPPORTED));
    }

    #[test]
    fn stream_columns_answer_from_the_facts() {
        let device = Arc::new(Logging::default());
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(device.clone());
        let state = Mutex::new(st);
        let ask = |payload: &[u8]| {
            handle(
                &state,
                1,
                &Request {
                    id: 1,
                    kind: kind::STREAM_COLUMNS,
                    payload,
                },
            )
        };
        // The ET5's answer for 0x504 (stream-probe, 2026-10-13).
        let presence = vec![
            request::StreamColumn {
                key: 1,
                name: "Timestamp".into(),
                param_b: 1,
                ..request::StreamColumn::default()
            },
            request::StreamColumn {
                key: 2,
                name: "Presence enum value".into(),
                param_b: 1,
                ..request::StreamColumn::default()
            },
        ];
        lock_state(&state).facts = Some(Arc::new(DeviceFacts {
            stream_columns: vec![
                (0x504, Some(presence.clone())),
                // Invented: an answer that lists none, and a refused one.
                (0x1770, Some(Vec::new())),
                (0x508, None),
            ],
            ..DeviceFacts::default()
        }));

        assert_eq!(
            ask(&[0x04, 0x05, 0x00]),
            Reply::err(status::INVALID_PARAMETER),
            "no stream id"
        );
        assert!(
            !lock_state(&state).clients[0].holds_device,
            "refused before the facts are waited for"
        );

        let reply = ask(&encode_u32(0x504));

        assert_eq!(reply, Reply::ok(encode_stream_columns(&presence)));
        assert!(lock_state(&state).clients[0].holds_device);
        assert_eq!(
            ask(&[0x04, 0x05, 0x00, 0x00, 0x09]),
            reply,
            "a tail is ignored"
        );
        assert_eq!(
            ask(&encode_u32(0x1770)),
            Reply::ok(encode_stream_columns(&[])),
            "none listed"
        );
        assert_eq!(
            ask(&encode_u32(0x508)),
            Reply::err(status::NOT_SUPPORTED),
            "refused"
        );
        assert_eq!(
            ask(&encode_u32(0x500)),
            Reply::err(status::NOT_SUPPORTED),
            "not asked, or its answer lost"
        );
        assert!(
            device.0.lock().expect("log").is_empty(),
            "no 1210, nor any other command"
        );
    }

    #[test]
    fn fault_and_warning_states_answer_from_the_last_init() {
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(Arc::new(Answering(1)));
        let state = Mutex::new(st);
        let ask = |id| {
            handle(
                &state,
                1,
                &Request {
                    id: 1,
                    kind: kind::STATE,
                    payload: &encode_u32(id),
                },
            )
        };
        let with_status = |status: &[(u32, &str)]| {
            lock_state(&state).facts = Some(Arc::new(DeviceFacts {
                status: status.iter().map(|(i, s)| (*i, (*s).to_owned())).collect(),
                ..DeviceFacts::default()
            }));
        };

        // The lists as the ET5 reports them in every Windows capture
        // (abridged).
        with_status(&[(4, "HELLO_RGB"), (5, "ok"), (6, "ok"), (7, "1904654973")]);
        assert_eq!(ask(state::FAULT), Reply::ok(b"ok".to_vec()));
        assert_eq!(ask(state::WARNING), Reply::ok(b"ok".to_vec()));
        assert!(lock_state(&state).clients[0].holds_device);

        // Invented lists, to tell the indices apart.
        with_status(&[(5, "FAULT_A,FAULT_B"), (6, "WARNING_A")]);
        assert_eq!(ask(state::FAULT), Reply::ok(b"FAULT_A,FAULT_B".to_vec()));
        assert_eq!(ask(state::WARNING), Reply::ok(b"WARNING_A".to_vec()));

        // An init whose 1490 left the lists out, and one whose 1490 was lost.
        for lists in [&[(7, "1904654973")][..], &[]] {
            with_status(lists);
            assert_eq!(ask(state::FAULT), Reply::err(status::NOT_SUPPORTED));
            assert_eq!(ask(state::WARNING), Reply::err(status::NOT_SUPPORTED));
        }
    }

    /// A list the tracker announces (3200, 3210) replaces the one its last
    /// init reported, until the next init reports its own; a list that init
    /// left out stays left out, as in the DLL.
    #[test]
    fn fault_and_warning_notifications_replace_the_reported_lists() {
        use tobii_proto::facts::DeviceNotification::{FaultsChanged, WarningsChanged};
        use tobii_usb::engine::Sample;
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(Arc::new(Answering(1)));
        let state = Mutex::new(st);
        let ask = |id| {
            handle(
                &state,
                1,
                &Request {
                    id: 1,
                    kind: kind::STATE,
                    payload: &encode_u32(id),
                },
            )
        };
        let facts = |status: &[(u32, &str)]| {
            Arc::new(DeviceFacts {
                status: status.iter().map(|(i, s)| (*i, (*s).to_owned())).collect(),
                ..DeviceFacts::default()
            })
        };
        let observe = |sample| lock_state(&state).observe(&sample);
        observe(Sample::DeviceReady(facts(&[(5, "ok"), (6, "ok")])));

        observe(Sample::Notification(FaultsChanged("FAULT_A".into())));

        assert_eq!(ask(state::FAULT), Reply::ok(b"FAULT_A".to_vec()));
        assert_eq!(ask(state::WARNING), Reply::ok(b"ok".to_vec()));

        observe(Sample::Notification(WarningsChanged("WARNING_A".into())));
        assert_eq!(ask(state::WARNING), Reply::ok(b"WARNING_A".to_vec()));
        assert_eq!(ask(state::FAULT), Reply::ok(b"FAULT_A".to_vec()));

        observe(Sample::DeviceReady(facts(&[(5, "ok"), (6, "WARNING_B")])));
        assert_eq!(
            ask(state::FAULT),
            Reply::ok(b"ok".to_vec()),
            "the new init's"
        );
        assert_eq!(ask(state::WARNING), Reply::ok(b"WARNING_B".to_vec()));

        observe(Sample::DeviceReady(facts(&[(7, "1904654973")])));
        observe(Sample::Notification(FaultsChanged("FAULT_A".into())));
        observe(Sample::Notification(WarningsChanged("WARNING_A".into())));
        assert_eq!(ask(state::FAULT), Reply::err(status::NOT_SUPPORTED));
        assert_eq!(ask(state::WARNING), Reply::err(status::NOT_SUPPORTED));
    }

    /// Power save and remote wake answer from the last init's 1490, and
    /// power save also from the 3020/3030 since, as the DLL's getters read a
    /// byte only a 1490 marks present and only those notifications rewrite.
    #[test]
    fn power_save_and_remote_wake_answer_from_the_1490_and_3020_3030() {
        use tobii_proto::facts::DeviceNotification::PowerSaveChanged;
        use tobii_usb::engine::Sample;
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(Arc::new(Answering(1)));
        let state = Mutex::new(st);
        let ask = |id| {
            handle(
                &state,
                1,
                &Request {
                    id: 1,
                    kind: kind::STATE,
                    payload: &encode_u32(id),
                },
            )
        };
        let facts = |status: &[(u32, &str)]| {
            Arc::new(DeviceFacts {
                status: status.iter().map(|(i, s)| (*i, (*s).to_owned())).collect(),
                ..DeviceFacts::default()
            })
        };
        let observe = |sample| lock_state(&state).observe(&sample);

        // As the ET5's 1490 reports both.
        observe(Sample::DeviceReady(facts(&[(0, "false"), (1, "false")])));
        assert_eq!(ask(state::POWER_SAVE_ACTIVE), Reply::ok(vec![0]));
        assert_eq!(ask(state::REMOTE_WAKE_ACTIVE), Reply::ok(vec![0]));

        // The 3020 the ET5 sent after a 3000, and the 3030 after the 3010.
        observe(Sample::Notification(PowerSaveChanged(true)));
        assert_eq!(ask(state::POWER_SAVE_ACTIVE), Reply::ok(vec![1]));
        assert_eq!(
            ask(state::REMOTE_WAKE_ACTIVE),
            Reply::ok(vec![0]),
            "nothing but a 1490 writes remote wake"
        );
        observe(Sample::Notification(PowerSaveChanged(false)));
        assert_eq!(ask(state::POWER_SAVE_ACTIVE), Reply::ok(vec![0]));

        // An init whose 1490 left them out: a notification adds nothing.
        observe(Sample::DeviceReady(facts(&[(7, "1904654973")])));
        observe(Sample::Notification(PowerSaveChanged(true)));
        assert_eq!(
            ask(state::POWER_SAVE_ACTIVE),
            Reply::err(status::NOT_SUPPORTED)
        );
        assert_eq!(
            ask(state::REMOTE_WAKE_ACTIVE),
            Reply::err(status::NOT_SUPPORTED)
        );

        // A value that is not a boolean is no value at all to the DLL, and a
        // notification does not make it one.
        observe(Sample::DeviceReady(facts(&[(1, "HELLO_RGB")])));
        assert_eq!(
            ask(state::POWER_SAVE_ACTIVE),
            Reply::err(status::NOT_SUPPORTED)
        );
        observe(Sample::Notification(PowerSaveChanged(true)));
        assert_eq!(
            ask(state::POWER_SAVE_ACTIVE),
            Reply::err(status::NOT_SUPPORTED)
        );
    }

    #[test]
    fn device_info_answers_from_the_facts() {
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(Arc::new(Answering(1)));
        let state = Mutex::new(st);
        let ask = || {
            handle(
                &state,
                1,
                &Request {
                    id: 1,
                    kind: kind::DEVICE_INFO,
                    payload: &[],
                },
            )
        };
        let with_properties = |properties: &[(u32, &str)]| {
            lock_state(&state).facts = Some(Arc::new(DeviceFacts {
                info: request::DeviceInfo {
                    model: "IS5_Large_Eyetracker_5".into(),
                    ..request::DeviceInfo::default()
                },
                properties: properties
                    .iter()
                    .map(|(i, s)| (*i, (*s).to_owned()))
                    .collect(),
                ..DeviceFacts::default()
            }));
        };

        // What the ET5 reports in both Windows captures (abridged): no
        // property 4, and property 5 -- which `tobii_get_gaze_hid_enabled`
        // reads -- as `false`.
        with_properties(&[(0, "Peripheral"), (3, "IS5LEYETRACKER5"), (5, "false")]);
        let reply = ask();

        assert_eq!(reply.status, status::OK);
        let info = request::decode_device_info(&reply.payload).expect("device info");
        assert_eq!(info.model, "IS5_Large_Eyetracker_5");
        assert_eq!(info.integration_type, "Peripheral");
        assert_eq!(
            info.internal,
            Some(request::DeviceInfoInternal {
                platform_type: "IS5LEYETRACKER5".into(),
                property_4: String::new(),
                property_5: "false".into(),
            })
        );
        assert!(lock_state(&state).clients[0].holds_device);

        // A first init whose 1330 was lost (nothing earlier to keep).
        with_properties(&[]);
        let reply = ask();
        assert_eq!(reply.status, status::OK);
        let info = request::decode_device_info(&reply.payload).expect("device info");
        assert_eq!(info.model, "IS5_Large_Eyetracker_5");
        assert_eq!(info.integration_type, "");
        assert_eq!(
            info.internal,
            Some(request::DeviceInfoInternal::default()),
            "empty, but there: this daemon knows them"
        );
    }

    #[test]
    fn the_hardware_configuration_answers_from_the_facts() {
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(Arc::new(Answering(1)));
        st.facts = Some(Arc::new(DeviceFacts::default()));
        let state = Mutex::new(st);
        let req = Request {
            id: 1,
            kind: kind::HARDWARE_CONFIGURATION,
            payload: &[],
        };

        // What the ET5 reports on Linux: an empty 2120.
        assert_eq!(handle(&state, 1, &req), Reply::err(status::NOT_SUPPORTED));
        assert!(lock_state(&state).clients[0].holds_device);

        let hardware = request::HardwareConfiguration {
            entries: vec![request::HardwareEntry {
                param_a: 16.0,
                width: 2240,
                ..request::HardwareEntry::default()
            }],
            points_mm: vec![[130.0, 0.76, 1.62]],
            mode: 1,
        };
        lock_state(&state).facts = Some(Arc::new(DeviceFacts {
            hardware: Some(hardware.clone()),
            ..DeviceFacts::default()
        }));
        let reply = handle(&state, 1, &req);

        assert_eq!(reply.status, status::OK);
        assert_eq!(
            request::decode_hardware_configuration(&reply.payload),
            Some(hardware)
        );
    }

    /// Ask for a clock pair from a state that has seen a gaze frame with
    /// `clock`, with a stand-in device for the running engine; `feed` runs
    /// beside the request as the pump would, from when the request has taken
    /// its device until it is answered.
    fn timesync_from(
        clock: Option<(i64, i64)>,
        feed: impl Fn(&Mutex<State>) + Send + Sync,
    ) -> (Reply, Mutex<State>) {
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(Arc::new(Answering(1)));
        if let Some((device_us, host_us)) = clock {
            st.note_clock(device_us, host_us);
        }
        let state = Mutex::new(st);
        let req = Request {
            id: 1,
            kind: kind::TIMESYNC,
            payload: &[],
        };
        let answered = AtomicBool::new(false);
        let reply = thread::scope(|s| {
            s.spawn(|| {
                while !answered.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(10));
                    if lock_state(&state).clients[0].holds_device {
                        feed(&state);
                    }
                }
            });
            let reply = handle(&state, 1, &req);
            answered.store(true, Ordering::Relaxed);
            reply
        });
        (reply, state)
    }

    #[test]
    fn timesync_serves_the_first_gaze_frame_after_the_request() {
        let asked = host_clock_us();
        let device_us = AtomicI64::new(5_000_000);
        // The gaze stream: a newer frame every 10 ms.
        let (reply, state) = timesync_from(Some((4_000_000, 1)), |state| {
            let device = device_us.fetch_add(10_000, Ordering::Relaxed);
            lock_state(state).note_clock(device, host_clock_us());
        });

        assert_eq!(reply.status, status::OK);
        let sync = request::decode_timesync(&reply.payload).expect("timesync");
        assert!(sync.device_us >= 5_000_000, "not the stale pair");
        assert!(sync.host_end_us >= asked);
        assert_eq!(sync.host_end_us - sync.host_start_us, FRAME_LATENCY_US);
        assert!(
            lock_state(&state).clients[0].holds_device,
            "the device stays up for the client"
        );
    }

    #[test]
    fn timesync_serves_a_new_frame_read_before_the_request() {
        // Read before the request and handed over after it: the frame's host
        // time is older than the request, here by far more than any latency
        // bound a host-time check could allow, but it is still new.
        let host_us = host_clock_us() - 10 * FRAME_LATENCY_US;
        let (reply, _) = timesync_from(Some((4_000_000, 1)), |state| {
            lock_state(state).note_clock(5_000_000, host_us);
        });

        assert_eq!(reply.status, status::OK);
        let sync = request::decode_timesync(&reply.payload).expect("timesync");
        assert_eq!((sync.device_us, sync.host_end_us), (5_000_000, host_us));
    }

    #[test]
    fn timesync_times_out_on_a_stale_pair() {
        let stale = |_: &Mutex<State>| {};
        let (reply, _) = timesync_from(Some((5_000_000, 1)), stale);
        assert_eq!(reply, Reply::err(status::TIMED_OUT));
        let (reply, _) = timesync_from(None, stale);
        assert_eq!(reply, Reply::err(status::TIMED_OUT));
    }

    #[test]
    fn timesync_fails_at_once_without_a_tracker() {
        // No stand-in device, and no tracker on the bus.
        let state = Mutex::new(crate::daemon::tests::state_with_client(1));
        let req = Request {
            id: 1,
            kind: kind::TIMESYNC,
            payload: &[],
        };

        let reply = handle(&state, 1, &req);

        assert_eq!(reply, Reply::err(status::CONNECTION_FAILED));
        let st = lock_state(&state);
        assert_eq!(
            st.presence_probes, 1,
            "the start's look at the bus: the answer did not wait for another"
        );
        assert!(
            st.engines_started.is_empty(),
            "no engine for an absent tracker"
        );
        assert!(
            st.clients[0].holds_device,
            "the watchdog starts it once it is plugged in"
        );
    }

    #[test]
    fn timesync_fails_when_the_tracker_is_unplugged_during_the_wait() {
        // Unplugged during the wait: the engine goes, and nothing is on the
        // bus.
        let (reply, state) = timesync_from(None, |state| lock_state(state).fake_device = None);

        assert_eq!(reply, Reply::err(status::CONNECTION_FAILED));
        assert_eq!(
            lock_state(&state).presence_probes,
            1,
            "one look, once the grace was over"
        );
    }

    /// The watchdog's restart, for a feed: the stand-in streams again a
    /// tick after the request looked for the tracker, by when a request that
    /// looked at every poll would have looked again.
    fn restart_after_the_look(st: &mut State, ticks_since_the_look: &AtomicU32) {
        if st.presence_probes > 0
            && st.fake_device.is_none()
            && ticks_since_the_look.fetch_add(1, Ordering::Relaxed) > 0
        {
            st.fake_device = Some(Arc::new(Answering(1)));
            st.note_clock(5_000_000, 1);
        }
    }

    #[test]
    fn timesync_waits_for_a_restart_while_the_tracker_is_plugged_in() {
        let since_the_look = AtomicU32::new(0);
        let (reply, state) = timesync_from(None, |state| {
            let mut st = lock_state(state);
            if st.presence_probes == 0 {
                // The engine is lost with the tracker still on the bus. The
                // request took its device before the stand-in went.
                st.fake_device = None;
                st.fake_present = true;
            } else {
                // The request found the tracker and waited on.
                restart_after_the_look(&mut st, &since_the_look);
            }
        });

        assert_eq!(reply.status, status::OK);
        let st = lock_state(&state);
        assert_eq!(
            st.presence_probes, 1,
            "the request looked for the tracker once, after the grace"
        );
        assert!(
            st.engines_started.is_empty(),
            "nothing in the wait starts an engine"
        );
    }

    #[test]
    fn timesync_gives_a_re_plugged_tracker_the_grace() {
        let ticks = AtomicU32::new(0);
        let since_the_look = AtomicU32::new(0);
        let (reply, state) = timesync_from(None, |state| {
            let tick = ticks.fetch_add(1, Ordering::Relaxed);
            let mut st = lock_state(state);
            match tick {
                // Unplugged: the engine goes, and the tracker is off the bus.
                0 => st.fake_device = None,
                // Back on the bus within the grace.
                1 => st.fake_present = true,
                _ => restart_after_the_look(&mut st, &since_the_look),
            }
        });

        assert_eq!(
            reply.status,
            status::OK,
            "the tracker was back by the time the request looked"
        );
        let st = lock_state(&state);
        assert_eq!(st.presence_probes, 1, "one look, once the grace was over");
        assert!(st.engines_started.is_empty());
    }

    #[test]
    fn requests_that_need_the_tracker_fail_at_once_without_one() {
        // No stand-in device, and no tracker on the bus.
        let mut st = crate::daemon::tests::state_with_client(1);
        st.calibration.location = tobii_calib::store::Location::Embedded;
        let state = Mutex::new(st);
        let area = encode_display_area(&DisplayArea {
            top_left_mm: [-300.0, 300.0, 0.0],
            top_right_mm: [300.0, 300.0, 0.0],
            bottom_left_mm: [-300.0, 0.0, 0.0],
        });
        let custom = request::encode_stream_start(0x1770, false);

        for (what, k, payload) in [
            ("a pause", kind::DEVICE_PAUSE, &[1][..]),
            ("a calibration", kind::CALIBRATION_START, &[2][..]),
            ("a display area", kind::DISPLAY_AREA_SET, &area[..]),
            (
                "a catalogue stream",
                kind::CATALOGUE_STREAM_START,
                &custom[..],
            ),
        ] {
            let reply = handle(
                &state,
                1,
                &Request {
                    id: 1,
                    kind: k,
                    payload,
                },
            );
            assert_eq!(reply, Reply::err(status::CONNECTION_FAILED), "{what}");
        }

        let st = lock_state(&state);
        assert!(
            st.engines_started.is_empty(),
            "no engine for an absent tracker"
        );
        assert!(
            !st.paused && !st.pausing && st.pause_hint.is_none(),
            "no pause is taken"
        );
        assert!(!st.calibration.is_active(), "nor a calibration session");
        assert_eq!(st.display_override, None, "nor a display area");
        assert!(st.catalogue.held().is_empty(), "nor a catalogue stream");
    }

    /// Answers every command with `status`.
    pub(crate) struct Answering(pub(crate) u32);

    impl crate::device::DeviceCommands for Answering {
        fn run(
            &self,
            _cmd: u32,
            _payload: Vec<u8>,
            _timeout: Duration,
        ) -> Result<tobii_usb::engine::CommandResponse, tobii_usb::engine::CommandError> {
            Ok(tobii_usb::engine::CommandResponse {
                status: self.0,
                ..tobii_usb::engine::CommandResponse::ok(Vec::new())
            })
        }
    }

    /// Set `area` on a device answering `device_status`; the reply and
    /// what the daemon now configures at init.
    fn set_area(
        device_status: u32,
        path: &std::path::Path,
        area: &DisplayArea,
    ) -> (Reply, Option<DisplayArea>) {
        let mut st = crate::daemon::tests::state_with_client(1);
        st.fake_device = Some(Arc::new(Answering(device_status)));
        st.display_file = Some(path.to_path_buf());
        let state = Mutex::new(st);
        let payload = encode_display_area(area);
        let reply = handle(
            &state,
            1,
            &Request {
                id: 1,
                kind: kind::DISPLAY_AREA_SET,
                payload: &payload,
            },
        );
        (reply, lock_state(&state).display_override)
    }

    #[test]
    fn a_display_area_the_device_accepts_is_saved_and_restored_at_start() {
        let dir = std::env::temp_dir().join(format!("tobiid-area-{}", std::process::id()));
        let path = dir.join("display-area");
        let area = display_area_basic(
            597.0,
            336.0,
            1.0,
            &tobii_ipc::geometry::GeometryMounting {
                guides: 2,
                width_mm: 184.0,
                angle_deg: 20.0,
                external_offset_mm: [0.0, -0.16, 13.85],
                internal_offset_mm: [0.0, 5.38, 9.86],
            },
        );

        let (reply, configured) = set_area(2, &path, &area);
        assert_eq!(reply, Reply::err(status::OPERATION_FAILED));
        assert!(!path.exists(), "a refused area is not saved");
        assert_eq!(configured, None, "nor configured for the next init");
        // Saved as received: the wire carries f32.
        let (reply, configured) = set_area(1, &path, &area);
        let area = decode_display_area(&encode_display_area(&area)).expect("wire");
        assert_eq!((reply, configured), (Reply::ok(Vec::new()), Some(area)));
        assert_eq!(crate::display::load(&path).expect("load"), Some(area));

        let mut st = crate::daemon::tests::state_with_client(1);
        st.display_file = Some(path.clone());
        st.display_request = None;
        restore_saved_display_area(&mut st);
        assert_eq!(st.display_override, Some(area));

        let mut st = crate::daemon::tests::state_with_client(1);
        st.display_file = Some(path.clone());
        st.display_request = DisplaySize::parse("600x340");
        restore_saved_display_area(&mut st);
        assert_eq!(st.display_override, Some(area), "the saved area wins");

        // The same area again leaves the rollback copy alone.
        let prev = tobii_calib::store::previous_path(&path);
        let _ = std::fs::remove_file(&prev);
        assert_eq!(set_area(1, &path, &area).0, Reply::ok(Vec::new()));
        assert!(!prev.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Answers every command and remembers which it got.
    #[derive(Default)]
    struct Logging(Mutex<Vec<u32>>);

    impl crate::device::DeviceCommands for Logging {
        fn run(
            &self,
            cmd: u32,
            _payload: Vec<u8>,
            _timeout: Duration,
        ) -> Result<tobii_usb::engine::CommandResponse, tobii_usb::engine::CommandError> {
            self.0.lock().expect("log").push(cmd);
            Ok(tobii_usb::engine::CommandResponse::ok(Vec::new()))
        }
    }

    #[test]
    fn an_in_session_area_set_that_replaces_a_dead_engine_is_refused() {
        // A new engine starts for a tracker on the bus and none without one:
        // either way the session went with the old engine.
        for (tag, on_bus) in [("replaced", true), ("unplugged", false)] {
            let dir =
                std::env::temp_dir().join(format!("tobiid-area-lost-{tag}-{}", std::process::id()));
            let path = dir.join("display-area");
            let device = Arc::new(Logging::default());
            let requested = DisplaySize::parse("600x340");
            let mut st = crate::daemon::tests::state_with_client(1);
            st.fake_device = Some(device.clone());
            st.fake_present = on_bus;
            st.display_file = Some(path.clone());
            st.display_request = requested;
            st.calibration.location = tobii_calib::store::Location::Embedded;
            let state = Mutex::new(st);
            let ask = |kind, payload: &[u8]| {
                handle(
                    &state,
                    1,
                    &Request {
                        id: 1,
                        kind,
                        payload,
                    },
                )
            };
            assert_eq!(ask(kind::CALIBRATION_START, &[2]), Reply::ok(Vec::new()));
            device.0.lock().expect("log").clear();
            lock_state(&state).lose_engine_on_fetch = true;
            let area = DisplayArea {
                top_left_mm: [-300.0, 300.0, 0.0],
                top_right_mm: [300.0, 300.0, 0.0],
                bottom_left_mm: [-300.0, 0.0, 0.0],
            };

            let reply = ask(kind::DISPLAY_AREA_SET, &encode_display_area(&area));

            assert_eq!(reply, Reply::err(status::CALIBRATION_NOT_STARTED), "{tag}");
            assert!(
                device.0.lock().expect("log").is_empty(),
                "{tag}: nothing reaches the device"
            );
            assert!(!path.exists(), "{tag}: nor is saved");
            let st = lock_state(&state);
            assert_eq!(st.display_override, None, "{tag}: nor configured");
            assert_eq!(
                st.display_request, requested,
                "{tag}: TOBII_DISPLAY_MM still applies"
            );
            assert_eq!(st.engines_started.len(), usize::from(on_bus), "{tag}");
            drop(st);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_non_finite_display_area_is_rejected() {
        let state = Mutex::new(crate::daemon::tests::state_with_client(1));
        let area = DisplayArea {
            top_left_mm: [f64::NAN, 0.0, 0.0],
            ..DisplayArea::default()
        };
        let payload = encode_display_area(&area);
        let req = Request {
            id: 1,
            kind: kind::DISPLAY_AREA_SET,
            payload: &payload,
        };
        assert_eq!(
            handle(&state, 1, &req),
            Reply::err(status::INVALID_PARAMETER)
        );
    }
}
