//! `stream-probe`: start one stream from the tracker's own catalogue for a
//! few seconds and record what EP 0x83 sends around it.
//!
//! Besides the init it sends only what the Windows Stream Engine was seen
//! sending, and, with `--columns`, command 1210. The init is the one tobiid
//! sends (the user's saved calibration and display area substituted, as
//! tobiid does, unless `--init` names a capture to replay). Then come command
//! 1220 for the IR image stream 0x50e, which tobiid's engine runs beside gaze
//! (unless `--no-image`; tobiid starts it before gaze arms, the probe once
//! gaze has), 1220 for the probed stream, 1230 for each, and the vendor
//! control 66 that ends every open. Before its first command it checks the
//! stream against the catalogue the tracker answered the init's 1200 with,
//! and refuses one it does not list. EP 0x83 is read throughout, also while a
//! command waits: an unread IN message stalls the tracker's OUT endpoint.
//! Once a 1220 has been tried, its 1230 is sent whatever happens next, a
//! failing log or an interrupt (SIGINT, SIGTERM) included.
//!
//! With `--columns` it asks the tracker for the stream's columns, command
//! 1210, which no capture holds, after the window before and just before the
//! 1220. The DLL sends 1210 as a pure query, from
//! `tobii_enumerate_stream_type_columns` and from its own capability checks
//! (`tracker_enumerate_stream_type_columns`, 0x18019c9a0), for the device ids
//! its stream-type map can name (or 0); the probe refuses any other id.
//! `--no-start` sends no 1220 or 1230 for the probed stream (those of 0x50e
//! still go, unless `--no-image`), for a stream the init already runs (gaze
//! 0x500, presence 0x504), which a 1230 would stop: the probe refuses to
//! start or stop those.
//!
//! The report splits the run into three windows: before the 1220 (what runs
//! under tobiid), from the 1220 to the 1230, and after the 1230. Per window
//! it gives each stream's message count, rate and sizes, the gaze frames'
//! validity, the responses and notifications. The log holds every whole
//! message from the probe's first command on, each command at endpoint 0x05
//! where the device took it; the init and the wait for gaze to arm are not
//! logged, nor bytes the reassembler drops to find the next message.

use anyhow::{Context, Result, bail};
use rusb::Context as UsbContext;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use tobii_ipc::geometry::DisplayArea;
use tobii_ipc::request::StreamType;
use tobii_proto::facts::parse_stream_catalogue;
use tobii_proto::gaze83::decode_gaze_frame;
use tobii_proto::log::{PacketLog, log_packet};
use tobii_proto::protocol::{
    BulkReassembler, InitPacket, MARKER_NOTIFICATION, MARKER_RESPONSE, MARKER_STREAM,
    STREAM_ID_GAZE, STREAM_ID_IMAGE, STREAM_ID_PRESENCE, chunk_command, cmd, parse_message,
    read_init_packets, stream_start_packet, stream_stop_packet,
};
use tobii_proto::tlv::{TYPE_U32, Tlv};
use tobii_usb::device::{
    EP_IN, EP_OUT, Observed, READ_BUF, next_command_seq, open_tobii, replay_init_packets,
    send_command_observing, vendor_control_deinit, vendor_control_init, wait_for_gaze_stream,
};
use tracing::warn;

use crate::cli::Command;

/// Opens tried before giving up: the first open after a cold start may not
/// arm the gaze stream (see `image83`).
const OPEN_ATTEMPTS: u32 = 3;

/// How long an open waits for its first gaze frame.
const ARM_TIMEOUT: Duration = Duration::from_millis(4500);

/// How long a command waits for its response once written.
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(1500);

/// One read of EP 0x83 inside a window.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

/// File name of the saved display area in the per-user Tobii directory
/// (tobiid's `display.rs`).
const DISPLAY_AREA_FILE: &str = "display-area";

/// Gaze below this share of its rate before the 1220 in the window after the
/// 1230 is reported as a gaze stream that did not come back.
const GAZE_RECOVERED_SHARE: f64 = 0.8;

/// Shortest window whose gaze rate the report compares: a shorter one may
/// hold no gaze frame at all.
const RATE_WINDOW_MIN: Duration = Duration::from_secs(1);

/// Set by the SIGINT/SIGTERM handler: the probe ends its windows, sends the
/// stops and ends the open, rather than leave the streams running. A pure
/// signal, so every access is `Ordering::Relaxed`.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_sig: libc::c_int) {
    // Async-signal-safe: only an atomic store.
    INTERRUPTED.store(true, Ordering::Relaxed);
}

/// Have SIGINT and SIGTERM set [`INTERRUPTED`] instead of ending the process.
///
/// # Errors
///
/// The OS error, should a handler not be installed.
fn catch_interrupts() -> Result<()> {
    let handler: extern "C" fn(libc::c_int) = on_interrupt;
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: `handler` is an `extern "C" fn(c_int)`, what `sighandler_t`
        // stands for, and only performs an atomic store, so it is
        // async-signal-safe; the default handler it replaces needs no
        // restoring.
        let previous = unsafe { libc::signal(signal, handler as libc::sighandler_t) };
        if previous == libc::SIG_ERR {
            return Err(std::io::Error::last_os_error()).context("catching SIGINT and SIGTERM");
        }
    }
    Ok(())
}

/// Command 1210: the columns of a stream (the DLL's
/// `tracker_enumerate_stream_type_columns`, 0x18019c9a0).
const STREAM_COLUMNS: u32 = 1210;

/// The device stream ids the DLL can ask 1210 for: the image of its map from
/// Stream Engine stream type to device id (0x18015e580, the switch at
/// 0x18019d018, then 0x1801801a0). The other types send id 0, but 10, which
/// is refused first (0x180176fac).
const COLUMN_IDS: [u32; 10] = [
    0x500, 0x501, 0x502, 0x504, 0x505, 0x506, 0x507, 0x508, 0x50a, 0x1770,
];

/// Streams the init replay starts itself: a 1230 for one would stop it.
const INIT_STREAMS: [u32; 2] = [STREAM_ID_GAZE, STREAM_ID_PRESENCE];

/// Field id of a column entry in a 1210 answer (the DLL's reader at
/// 0x180004a80).
const FIELD_COLUMN_ENTRY: u32 = 0x0005_1388;

/// Low half of a list header's field id; the high half's low 12 bits are the
/// entry count plus one.
const FIELD_LIST: u32 = 0x0100;

/// The `stream-probe` subcommand.
///
/// # Errors
///
/// Fails when the plan sends what the probe must not (see `Plan::check`),
/// the init cannot be built, the tracker cannot be opened or does not list
/// the stream, the gaze stream never arms, the run is interrupted, the log
/// cannot be written, or a transfer fails. On every path past a 1220 its
/// 1230 is sent and the open ended with control 66, and the report of what
/// came until then is printed.
///
/// # Panics
///
/// Panics if `command` is not `Command::StreamProbe`; `main.rs` dispatches
/// on the variant before calling this.
pub(crate) fn run(command: &Command) -> Result<()> {
    let Command::StreamProbe {
        id,
        init_path,
        pre_secs,
        secs,
        post_secs,
        log_path,
        columns,
        start,
        image,
    } = command
    else {
        unreachable!();
    };
    let plan = Plan {
        id: *id,
        columns: *columns,
        start: *start,
        image: *image,
    };
    plan.check()?;
    let packets = match init_path {
        Some(path) => read_init_packets(path)?,
        None => daemon_init_packets()?,
    };
    let mut log = Some(PacketLog::create(log_path)?);
    let windows = [*pre_secs, *secs, *post_secs].map(Duration::from_secs_f64);
    catch_interrupts()?;
    let ctx = UsbContext::new()?;
    for attempt in 1..=OPEN_ATTEMPTS {
        if INTERRUPTED.load(Ordering::Relaxed) {
            bail!("interrupted before the probe began");
        }
        let mut h = open_tobii(&ctx)?;
        vendor_control_init(&mut h)?;
        println!("open {attempt}: replaying {} init packets", packets.len());
        let capture = match replay_init_packets(&mut h, &packets, Some(&INTERRUPTED)) {
            Ok(capture) => capture,
            Err(e) => {
                vendor_control_deinit(&mut h);
                return Err(e);
            }
        };
        let catalogue = catalogue_of(&capture.responses);
        if let Err(e) = ensure_listed(&catalogue, *id) {
            vendor_control_deinit(&mut h);
            return Err(e);
        }
        let mut asm = BulkReassembler::new();
        if !wait_for_gaze_stream(&mut h, &INTERRUPTED, ARM_TIMEOUT, &mut asm) {
            println!("no gaze frame within {ARM_TIMEOUT:?}; re-opening (cold-start quirk)");
            vendor_control_deinit(&mut h);
            drop(h);
            thread::sleep(Duration::from_millis(700));
            continue;
        }
        let mut seq = next_command_seq(&packets);
        let (report, outcome) = probe(&mut h, &mut asm, &mut log, plan, &mut seq, windows);
        vendor_control_deinit(&mut h);
        print!("{}", report.render());
        println!("log: {log_path}");
        if INTERRUPTED.load(Ordering::Relaxed) {
            bail!("interrupted: the windows were cut short, the stops sent");
        }
        return outcome;
    }
    bail!("the gaze stream never armed in {OPEN_ATTEMPTS} opens")
}

/// The init tobiid sends: the embedded capture with the user's saved
/// calibration and display area substituted.
fn daemon_init_packets() -> Result<Vec<InitPacket>> {
    let area = tobii_calib::store::config_dir()
        .and_then(|dir| saved_display_area(&dir.join(DISPLAY_AREA_FILE)));
    match &area {
        Some(_) => println!("init: the user's calibration and saved display area"),
        None => println!("init: the user's calibration and the capture's display area"),
    }
    tobii_usb::calibration::init_packets(area.as_ref())
}

/// The display area tobiid saved at `path` (the text its `display.rs`
/// writes: `top_left`, `top_right` and `bottom_left`, three finite numbers
/// each, in mm); `None` when there is no file, and, as tobiid at its start,
/// with a warning when it cannot be read or holds no area.
fn saved_display_area(path: &Path) -> Option<DisplayArea> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "the saved display area cannot be read; ignored");
            return None;
        }
    };
    let area = display_area_from_text(&text);
    if area.is_none() {
        warn!(path = %path.display(), "the saved display area is not one; ignored");
    }
    area
}

/// Parse tobiid's display-area text as its `display.rs` does: each corner
/// once, three finite values, and a display with a width and a height.
fn display_area_from_text(text: &str) -> Option<DisplayArea> {
    let (mut tl, mut tr, mut bl) = (None, None, None);
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let slot = match words.next()? {
            "top_left" => &mut tl,
            "top_right" => &mut tr,
            "bottom_left" => &mut bl,
            _ => return None,
        };
        let mut p = [0.0; 3];
        for v in &mut p {
            *v = words
                .next()?
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())?;
        }
        if words.next().is_some() || slot.replace(p).is_some() {
            return None;
        }
    }
    let area = DisplayArea {
        top_left_mm: tl?,
        top_right_mm: tr?,
        bottom_left_mm: bl?,
    };
    let span = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>();
    (span(area.top_left_mm, area.top_right_mm) > 1.0
        && span(area.top_left_mm, area.bottom_left_mm) > 1.0)
        .then_some(area)
}

/// The catalogue among an init's responses (its 1200 answer, parsed as the
/// facts parse it); empty when the init got none.
fn catalogue_of(responses: &[Vec<u8>]) -> Vec<StreamType> {
    responses
        .iter()
        .filter_map(|r| parse_message(r))
        .find(|m| m.marker == MARKER_RESPONSE && m.id == cmd::STREAM_CATALOGUE)
        .map(|m| parse_stream_catalogue(&m))
        .unwrap_or_default()
}

/// Refuse a stream the tracker's own catalogue does not list.
///
/// # Errors
///
/// When `id` is not in `catalogue` (an empty one included: the init's 1200
/// went unanswered).
fn ensure_listed(catalogue: &[StreamType], id: u32) -> Result<()> {
    match catalogue.iter().find(|t| t.id == id) {
        Some(t) => {
            println!(
                "stream {id:#x} \"{}\" (value {}) is in the tracker's catalogue of {}",
                t.name,
                t.value,
                catalogue.len()
            );
            Ok(())
        }
        None => {
            let listed: Vec<String> = catalogue.iter().map(|t| format!("{:#x}", t.id)).collect();
            bail!(
                "stream {id:#x} is not in the tracker's catalogue [{}]; nothing sent besides the init",
                listed.join(", ")
            )
        }
    }
}

/// What one run sends besides the init.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Plan {
    /// The probed stream.
    id: u32,
    /// Ask its columns (1210), after the window before, just before the 1220.
    columns: bool,
    /// Start it (1220) and stop it (1230).
    start: bool,
    /// Run the IR image stream 0x50e throughout, as tobiid's engine does.
    image: bool,
}

impl Plan {
    /// Refuse what the probe must not send, before the tracker is opened.
    ///
    /// # Errors
    ///
    /// A 1210 for an id the DLL cannot ask it for, a 1220/1230 for a stream
    /// the init already runs, a run that sends nothing, or 0x50e probed while
    /// the probe runs it as the baseline.
    fn check(self) -> Result<()> {
        if self.columns && !COLUMN_IDS.contains(&self.id) {
            bail!(
                "the DLL never asks 1210 for {:#x}; --columns takes only {}",
                self.id,
                COLUMN_IDS.map(|i| format!("{i:#x}")).join(", ")
            );
        }
        if self.start && INIT_STREAMS.contains(&self.id) {
            bail!(
                "{:#x} runs from the init on; its 1230 would stop it (use --no-start)",
                self.id
            );
        }
        if !self.columns && !self.start {
            bail!("--no-start without --columns sends nothing");
        }
        if self.image && self.id == STREAM_ID_IMAGE {
            bail!("probing 0x50e itself needs --no-image");
        }
        Ok(())
    }
}

/// The probe proper, on an armed open: the image stream's start if asked,
/// the window before, the 1210 if asked, the 1220, the window it runs, the
/// 1230 (sent whatever happened since the 1220), the window after, and the
/// image stream's stop (sent whatever happened since its start). Returns
/// what came until the first error, and the error.
fn probe(
    h: &mut rusb::DeviceHandle<UsbContext>,
    asm: &mut BulkReassembler,
    log: &mut Option<PacketLog>,
    plan: Plan,
    seq: &mut u32,
    windows: [Duration; 3],
) -> (Report, Result<()>) {
    let mut report = Report::new(plan.id);
    let mut outcome = Ok(());
    if plan.image {
        // As tobiid's engine runs it beside gaze: what runs every day.
        let packet = stream_start_packet(*seq, STREAM_ID_IMAGE);
        *seq += 1;
        match command(h, asm, log, &mut report.windows[0], &packet) {
            Ok(rsp) => report.image_start = Some(rsp),
            Err(e) => outcome = Err(e.context("starting the image stream 0x50e")),
        }
    }
    if outcome.is_ok() {
        outcome = probe_stream(h, asm, log, plan, seq, windows, &mut report);
    }
    if plan.image {
        // Whatever came of its start, which may have reached the tracker
        // before failing. Best effort, as the engine's: control 66 ends
        // every stream anyway.
        let packet = stream_stop_packet(*seq, STREAM_ID_IMAGE);
        *seq += 1;
        match command(h, asm, log, &mut report.windows[2], &packet) {
            Ok(rsp) => report.image_stop = Some(rsp),
            Err(e) => warn!(error = %format!("{e:#}"), "stopping the image stream 0x50e"),
        }
    }
    (report, outcome)
}

/// [`probe`] past the image stream's start.
fn probe_stream(
    h: &mut rusb::DeviceHandle<UsbContext>,
    asm: &mut BulkReassembler,
    log: &mut Option<PacketLog>,
    plan: Plan,
    seq: &mut u32,
    [pre, during, post]: [Duration; 3],
    report: &mut Report,
) -> Result<()> {
    read_window(h, asm, log, &mut report.windows[0], pre)?;
    if plan.columns {
        let packet = stream_columns_packet(*seq, plan.id)?;
        *seq += 1;
        report.columns = Some(command(h, asm, log, &mut report.windows[0], &packet)?);
    }
    if INTERRUPTED.load(Ordering::Relaxed) {
        return Ok(());
    }
    if plan.start {
        let started = command(
            h,
            asm,
            log,
            &mut report.windows[1],
            &stream_start_packet(*seq, plan.id),
        );
        *seq += 1;
        let ran = match &started {
            Ok(_) => read_window(h, asm, log, &mut report.windows[1], during),
            Err(_) => Ok(()),
        };
        let stopped = command(
            h,
            asm,
            log,
            &mut report.windows[2],
            &stream_stop_packet(*seq, plan.id),
        );
        *seq += 1;
        report.start = Some(started?);
        ran?;
        report.stop = Some(stopped?);
    }
    read_window(h, asm, log, &mut report.windows[2], post)
}

/// Command 1210 for stream `id`, as the DLL builds it: `00 00`, then the id
/// as a type-2 `u32`, the payload of a 1230.
///
/// # Errors
///
/// Never in practice: a command this short is one packet.
fn stream_columns_packet(seq: u32, id: u32) -> Result<Vec<u8>> {
    let mut payload = vec![0x00, 0x00, TYPE_U32, 0x00, 0x00, 0x00, 0x04];
    payload.extend_from_slice(&id.to_be_bytes());
    chunk_command(STREAM_COLUMNS, seq, &payload)
        .into_iter()
        .next()
        .context("a 1210 makes no packet")
}

/// One column of a 1210 answer. The wire order is `key`, `name`, `text`,
/// `second`, `third`; the DLL packs them as `{key, second, third,
/// name[64], text[64]}` (0x18017c22e..0x18017c26d) and hands that to its
/// receiver (0x18019cfca).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Column {
    /// The column key (the DLL looks for 0x17 and 0x18 in 0x500).
    key: u32,
    name: String,
    text: String,
    second: u32,
    third: u32,
}

/// The columns in a 1210 answer, `None` when it does not parse as the DLL
/// reads it (0x1800046b0, 0x180004a80).
fn parse_columns(msg: &[u8]) -> Option<Vec<Column>> {
    let m = parse_message(msg)?;
    let mut entries = m.tlvs();
    let header = entries.next()?.field_id()?;
    if header & 0xffff != FIELD_LIST {
        return None;
    }
    let count = ((header >> 16) & 0xfff).checked_sub(1)?;
    let typed_u32 = |t: Tlv<'_>| (t.typ == TYPE_U32).then(|| t.u32()).flatten();
    // The entry type: a type-2 u32 the DLL reads and does not check
    // (0x1800046ff, through 0x180003d80).
    typed_u32(entries.next()?)?;
    (0..count)
        .map(|_| {
            if entries.next()?.field_id()? != FIELD_COLUMN_ENTRY {
                return None;
            }
            Some(Column {
                key: typed_u32(entries.next()?)?,
                name: entries.next()?.string()?,
                text: entries.next()?.string()?,
                second: typed_u32(entries.next()?)?,
                third: typed_u32(entries.next()?)?,
            })
        })
        .collect()
}

/// Write `packet` and read until its response, tallying all that comes
/// meanwhile into `window`, and logging it with the command (at EP 0x05)
/// where the device took it. The log never holds a write back: a log that
/// fails is reported once the command has gone out.
fn command(
    h: &mut rusb::DeviceHandle<UsbContext>,
    asm: &mut BulkReassembler,
    log: &mut Option<PacketLog>,
    window: &mut Window,
    packet: &[u8],
) -> Result<Option<Vec<u8>>> {
    window.begin();
    let mut logged = Ok(());
    let response = send_command_observing(h, packet, RESPONSE_TIMEOUT, asm, |event| {
        let record = match event {
            Observed::Written => log_packet(log, EP_OUT, packet),
            Observed::Read(m) => {
                window.add(m);
                log_packet(log, EP_IN, m)
            }
        };
        if logged.is_ok() {
            logged = record;
        }
    });
    window.end();
    let response = response?;
    logged?;
    Ok(response)
}

/// Read EP 0x83 for `dur`, logging every message and tallying it.
fn read_window(
    h: &mut rusb::DeviceHandle<UsbContext>,
    asm: &mut BulkReassembler,
    log: &mut Option<PacketLog>,
    window: &mut Window,
    dur: Duration,
) -> Result<()> {
    let mut buf = vec![0u8; READ_BUF];
    let mut msgs = Vec::new();
    window.begin();
    let start = Instant::now();
    while start.elapsed() < dur && !INTERRUPTED.load(Ordering::Relaxed) {
        let n = match h.read_bulk(EP_IN, &mut buf, READ_TIMEOUT) {
            Ok(n) => n,
            Err(rusb::Error::Timeout) => continue,
            Err(e) => {
                window.end();
                return Err(e).context("reading EP 0x83 (did the tracker disconnect?)");
            }
        };
        asm.push_into(&buf[..n], &mut msgs);
        for m in &msgs {
            log_packet(log, EP_IN, m)?;
            window.add(m);
        }
    }
    window.end();
    Ok(())
}

/// What one stream sent in a window.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct StreamTally {
    messages: u64,
    bytes: u64,
    min_len: usize,
    max_len: usize,
}

/// What came in one window of the probe.
#[derive(Debug, Default)]
struct Window {
    /// Time spent reading, over every stretch the window was open.
    elapsed: Duration,
    /// Since when the current stretch has been open.
    open_since: Option<Instant>,
    streams: BTreeMap<u32, StreamTally>,
    gaze_frames: u64,
    gaze_valid: u64,
    /// `(command, seq, status, error, payload length)` of each response.
    responses: Vec<(u32, u32, u32, u32, usize)>,
    /// `(id, payload length)` of each notification.
    notifications: Vec<(u32, usize)>,
    /// Messages that do not parse or carry an unknown marker.
    other: u64,
}

impl Window {
    fn begin(&mut self) {
        self.open_since.get_or_insert_with(Instant::now);
    }

    fn end(&mut self) {
        if let Some(since) = self.open_since.take() {
            self.elapsed += since.elapsed();
        }
    }

    /// Tally one whole message.
    fn add(&mut self, msg: &[u8]) {
        let Some(m) = parse_message(msg) else {
            self.other += 1;
            return;
        };
        match m.marker {
            MARKER_STREAM => {
                let t = self.streams.entry(m.id).or_default();
                t.min_len = if t.messages == 0 {
                    msg.len()
                } else {
                    t.min_len.min(msg.len())
                };
                t.max_len = t.max_len.max(msg.len());
                t.messages += 1;
                t.bytes += u64::try_from(msg.len()).unwrap_or(u64::MAX);
                if m.id == STREAM_ID_GAZE
                    && let Some(f) = decode_gaze_frame(&m)
                {
                    self.gaze_frames += 1;
                    self.gaze_valid += u64::from(f.gaze.valid);
                }
            }
            MARKER_RESPONSE => {
                self.responses
                    .push((m.id, m.seq, m.status, m.error, m.payload.len()));
            }
            MARKER_NOTIFICATION => self.notifications.push((m.id, m.payload.len())),
            _ => self.other += 1,
        }
    }

    /// Messages per second of `t` over the window.
    fn rate(&self, t: &StreamTally) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs > 0.0 {
            t.messages as f64 / secs
        } else {
            0.0
        }
    }

    fn gaze_rate(&self) -> f64 {
        self.streams
            .get(&STREAM_ID_GAZE)
            .map_or(0.0, |t| self.rate(t))
    }
}

/// The probe's findings.
#[derive(Debug)]
struct Report {
    id: u32,
    /// Before the 1220, from it to the 1230, after the 1230.
    windows: [Window; 3],
    /// The 1220's response, if one came.
    start: Option<Option<Vec<u8>>>,
    /// The 1230's response, if one came.
    stop: Option<Option<Vec<u8>>>,
    /// The 1210's response, if one came.
    columns: Option<Option<Vec<u8>>>,
    /// The image stream's 1220 response, when it was started.
    image_start: Option<Option<Vec<u8>>>,
    /// Its 1230 response, when it was stopped.
    image_stop: Option<Option<Vec<u8>>>,
}

impl Report {
    fn new(id: u32) -> Self {
        Self {
            id,
            windows: Default::default(),
            start: None,
            stop: None,
            columns: None,
            image_start: None,
            image_stop: None,
        }
    }

    /// The report as printed.
    fn render(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "\n=== stream-probe {:#x} ===", self.id);
        if let Some(rsp) = &self.columns {
            let line = rsp
                .as_deref()
                .map_or_else(|| "no response".to_owned(), describe_response);
            let _ = writeln!(s, "1210 columns: {line}");
            match rsp.as_deref().map(parse_columns) {
                Some(Some(columns)) => {
                    for c in columns {
                        let _ = writeln!(
                            s,
                            "   column {:#06x} {:?} {:?} {:#x} {:#x}",
                            c.key, c.name, c.text, c.second, c.third
                        );
                    }
                }
                Some(None) => {
                    let _ = writeln!(s, "   the answer does not parse as the DLL reads it");
                }
                None => {}
            }
        }
        for (what, rsp) in [
            ("0x50e 1220 start", &self.image_start),
            ("0x50e 1230 stop", &self.image_stop),
        ] {
            if let Some(rsp) = rsp {
                let line = rsp
                    .as_deref()
                    .map_or_else(|| "no response".to_owned(), describe_response);
                let _ = writeln!(s, "{what}: {line}");
            }
        }
        for (what, rsp) in [("1220 start", &self.start), ("1230 stop", &self.stop)] {
            let line = match rsp {
                Some(Some(r)) => describe_response(r),
                Some(None) => "no response".to_owned(),
                None => "not sent".to_owned(),
            };
            let _ = writeln!(s, "{what}: {line}");
        }
        for (name, w) in ["before", "during", "after"].iter().zip(&self.windows) {
            let _ = writeln!(s, "-- {name} ({:.2} s)", w.elapsed.as_secs_f64());
            for (id, t) in &w.streams {
                let _ = writeln!(
                    s,
                    "   stream {id:#06x}: {:6} msgs {:7.2}/s  {:10} B  len {}..{}",
                    t.messages,
                    w.rate(t),
                    t.bytes,
                    t.min_len,
                    t.max_len
                );
            }
            if w.gaze_frames > 0 {
                let _ = writeln!(
                    s,
                    "   gaze valid {}/{} ({:.1} %)",
                    w.gaze_valid,
                    w.gaze_frames,
                    w.gaze_valid as f64 * 100.0 / w.gaze_frames as f64
                );
            }
            for (id, seq, status, error, len) in &w.responses {
                let _ = writeln!(
                    s,
                    "   response {id} seq {seq:#x}: status {status} error {error:#010x}, {len} B"
                );
            }
            for (id, len) in &w.notifications {
                let _ = writeln!(s, "   notification {id}: {len} B");
            }
            if w.other > 0 {
                let _ = writeln!(s, "   other messages: {}", w.other);
            }
        }
        let [before, _, after] = &self.windows;
        if before.elapsed >= RATE_WINDOW_MIN
            && after.elapsed >= RATE_WINDOW_MIN
            && after.gaze_rate() < before.gaze_rate() * GAZE_RECOVERED_SHARE
        {
            let _ = writeln!(
                s,
                "WARNING: gaze after the 1230 ran at {:.1}/s, before the 1220 at {:.1}/s",
                after.gaze_rate(),
                before.gaze_rate()
            );
        }
        s
    }
}

/// A response's status and TTP error words.
fn describe_response(msg: &[u8]) -> String {
    match parse_message(msg) {
        Some(m) => format!(
            "status {} error {:#010x}, {} B payload",
            m.status,
            m.error,
            m.payload.len()
        ),
        None => format!("{} bytes that do not parse", msg.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tobii_proto::protocol::{MARKER_COMMAND, command_message, hex_to_bytes};

    /// A device->host message: the 8-byte prefix, then `header` words and
    /// `payload`.
    fn message(marker: u32, seq: u32, status: u32, id: u32, error: u32, payload: &[u8]) -> Vec<u8> {
        let total = 32 + payload.len();
        let mut v = vec![1, 0, 0, 0];
        v.extend_from_slice(&u32::try_from(total).expect("small").to_le_bytes());
        for w in [marker, seq, status, id, error] {
            v.extend_from_slice(&w.to_be_bytes());
        }
        v.extend_from_slice(&u32::try_from(payload.len()).expect("small").to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn catalogue() -> Vec<StreamType> {
        [(0x500, "gaze", 0), (0x508, "image_collection", 1000)]
            .into_iter()
            .map(|(id, name, value)| StreamType {
                id,
                name: name.into(),
                text: String::new(),
                value,
            })
            .collect()
    }

    #[test]
    fn a_stream_the_tracker_lists_passes_the_gate() {
        assert!(ensure_listed(&catalogue(), 0x508).is_ok());
    }

    #[test]
    fn a_stream_the_tracker_does_not_list_is_refused() {
        let e = ensure_listed(&catalogue(), 0x502).expect_err("not listed");
        assert!(
            e.to_string().contains("not in the tracker's catalogue"),
            "{e}"
        );
        assert!(ensure_listed(&[], 0x500).is_err(), "no catalogue, no probe");
    }

    #[test]
    fn the_catalogue_comes_from_the_inits_1200_answer() {
        let bytes = hex_to_bytes(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tobii-proto/fixtures/init-rsp-1200.hex"
        )))
        .expect("fixture is valid hex");
        let other = message(MARKER_RESPONSE, 3, 1, 1330, 0, &[0, 0]);
        let catalogue = catalogue_of(&[other, bytes.clone()]);

        let ids: Vec<u32> = catalogue.iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            [
                0x500, 0x501, 0x504, 0x508, 0x50e, 0x1770, 0x1771, 0x1772, 0x1774
            ]
        );
        assert!(catalogue_of(&[]).is_empty());
        // A command (host->device) with the same id is not the answer.
        let asked = command_message(cmd::STREAM_CATALOGUE, 1, &[]);
        let mut prefixed = vec![0, 0, 0, 0, 0, 0, 0, 0];
        prefixed.extend_from_slice(&asked);
        assert_eq!(
            parse_message(&prefixed).map(|m| m.marker),
            Some(MARKER_COMMAND)
        );
        assert!(catalogue_of(&[prefixed]).is_empty());
    }

    #[test]
    fn a_window_tallies_streams_responses_and_notifications() {
        let mut w = Window::default();
        w.add(&message(MARKER_STREAM, 0, 0, 0x508, 0, &[0; 100]));
        w.add(&message(MARKER_STREAM, 0, 0, 0x508, 0, &[0; 40]));
        w.add(&message(MARKER_RESPONSE, 0x7a, 1, 1220, 0, &[0, 0]));
        w.add(&message(MARKER_NOTIFICATION, 0, 0, 3180, 0, &[0; 9]));
        w.add(&[1, 2, 3]);

        let t = &w.streams[&0x508];
        assert_eq!((t.messages, t.min_len, t.max_len), (2, 72, 132));
        assert_eq!(t.bytes, 204);
        assert_eq!(w.responses, [(1220, 0x7a, 1, 0, 2)]);
        assert_eq!(w.notifications, [(3180, 9)]);
        assert_eq!(w.other, 1);
        assert_eq!(w.gaze_frames, 0, "no gaze message, no gaze frame");
    }

    #[test]
    fn the_report_warns_when_gaze_does_not_come_back() {
        let gaze = message(MARKER_STREAM, 0, 0, STREAM_ID_GAZE, 0, &[0, 0]);
        let mut report = Report::new(0x501);
        for _ in 0..10 {
            report.windows[0].add(&gaze);
        }
        report.windows[2].add(&gaze);
        report.windows[0].elapsed = Duration::from_secs(1);
        report.windows[2].elapsed = Duration::from_secs(1);
        report.start = Some(None);

        let text = report.render();

        assert!(text.contains("WARNING: gaze after the 1230"), "{text}");
        assert!(text.contains("1220 start: no response"), "{text}");
        assert!(text.contains("1230 stop: not sent"), "{text}");
        // A window too short to hold a gaze frame tells nothing.
        report.windows[2].elapsed = Duration::from_millis(40);
        assert!(!report.render().contains("WARNING"), "a 40 ms window");
    }

    #[test]
    fn a_1210_is_a_1230_with_another_command() {
        let columns = stream_columns_packet(0x2c, 0x508).expect("packet");
        let mut stop = stream_stop_packet(0x2c, 0x508);
        // The command id word: after the 8-byte prefix, marker, seq, status.
        stop[20..24].copy_from_slice(&1210u32.to_be_bytes());

        assert_eq!(columns, stop);
        assert_eq!(
            &columns[columns.len() - 11..],
            &[0, 0, 2, 0, 0, 0, 4, 0, 0, 5, 8]
        );
    }

    #[test]
    fn the_plan_refuses_what_the_probe_must_not_send() {
        let plan = |id, columns, start| {
            Plan {
                id,
                columns,
                start,
                image: true,
            }
            .check()
        };

        assert!(plan(0x508, true, true).is_ok());
        assert!(plan(0x500, true, false).is_ok(), "gaze's columns, no start");
        assert!(plan(0x1770, false, true).is_ok());
        assert!(
            plan(0x50e, true, false).is_err(),
            "the DLL never asks 0x50e's columns"
        );
        assert!(plan(0x1774, true, true).is_err());
        assert!(
            plan(0x500, false, true).is_err(),
            "its 1230 would stop gaze"
        );
        assert!(plan(0x504, true, true).is_err(), "nor presence");
        assert!(plan(0x501, false, false).is_err(), "nothing to send");
        assert!(plan(0x50e, false, true).is_err(), "0x50e runs already");
        let without_image = Plan {
            id: 0x50e,
            columns: false,
            start: true,
            image: false,
        };
        assert!(without_image.check().is_ok(), "0x50e alone, as a control");
        // The 1210 gate on its own: without the image baseline, which
        // refuses 0x50e anyway, it still must not ask 0x50e's columns.
        let columns_alone = Plan {
            id: 0x50e,
            columns: true,
            start: false,
            image: false,
        };
        assert!(
            columns_alone.check().is_err(),
            "the DLL never asks 0x50e's columns, baseline or not"
        );
    }

    #[test]
    fn a_1210_answer_parses_as_the_dll_reads_it() {
        let mut w = tobii_proto::tlv::TlvWriter::default();
        // Two entries: the list header's count is n + 1.
        w.field_id((3 << 16) | FIELD_LIST).u32(0x1388);
        for (key, name) in [(0x17, "left_eyeball_center"), (0x18, "right")] {
            w.field_id(FIELD_COLUMN_ENTRY)
                .u32(key)
                .string(name)
                .string("")
                .u32(7)
                .u32(12);
        }
        let msg = message(MARKER_RESPONSE, 0x2c, 1, 1210, 0, &w.finish());

        let columns = parse_columns(&msg).expect("parses");

        assert_eq!(columns.len(), 2);
        assert_eq!(
            columns[0],
            Column {
                key: 0x17,
                name: "left_eyeball_center".into(),
                text: String::new(),
                second: 7,
                third: 12
            }
        );
        // The entry type must be a type-2 u32, as the DLL reads it.
        let mut w = tobii_proto::tlv::TlvWriter::default();
        w.field_id((1 << 16) | FIELD_LIST).field_id(0x1388);
        let untyped = message(MARKER_RESPONSE, 0x2c, 1, 1210, 0, &w.finish());
        assert_eq!(parse_columns(&untyped), None);
        // A wrong entry id fails the whole answer, as in the DLL.
        let mut bad = msg.clone();
        let at = bad
            .windows(4)
            .position(|w| w == FIELD_COLUMN_ENTRY.to_be_bytes())
            .expect("entry");
        bad[at + 3] ^= 1;
        assert_eq!(parse_columns(&bad), None);
    }

    #[test]
    fn the_saved_display_area_reads_as_tobiid_writes_it() {
        let text = "# comment\ntop_left -298.5 324.4 111.2\ntop_right 298.5 324.4 111.2\n\
                    bottom_left -298.5 10.3 -3.1\n";
        let area = display_area_from_text(text).expect("area");
        assert_eq!(area.top_right_mm, [298.5, 324.4, 111.2]);
        assert_eq!(area.bottom_left_mm, [-298.5, 10.3, -3.1]);
        assert!(display_area_from_text("top_left 0 0 0\n").is_none());
        assert!(display_area_from_text(&format!("{text}top_left 1 2 3\n")).is_none());
        assert!(display_area_from_text("top_left 0 0 nan\n").is_none());
        // A display with no width or height, which tobiid ignores too.
        let flat = "top_left 0 10 0\ntop_right 0.5 10 0\nbottom_left 0 0 0\n";
        assert!(display_area_from_text(flat).is_none());
    }
}
