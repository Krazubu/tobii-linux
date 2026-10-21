//! The device commands the daemon runs on behalf of clients, and the starts
//! and stops of the tracker's catalogue streams it makes for them, behind a
//! trait so the request handlers can be tested without a tracker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tobii_ipc::request::{status, tracker_error};
use tobii_proto::protocol::{RESPONSE_STATUS_OK, STREAM_ID_IMAGE_COLLECTION, cmd, ttp_error};
use tobii_usb::engine::{
    CommandError, CommandResponse, Commands, DeviceCommand, StreamAnswer, StreamChange,
};
use tracing::{Level, debug, info, warn};

/// Runs one device command and waits for the answer.
pub(crate) trait DeviceCommands: Send + Sync {
    /// Send `cmd` with `payload` and wait up to `timeout` for the response.
    fn run(
        &self,
        cmd: u32,
        payload: Vec<u8>,
        timeout: Duration,
    ) -> Result<CommandResponse, CommandError>;

    /// Start or stop a catalogue stream, its answer due within `timeout`,
    /// and hand back the answer to wait for: the engine queues the change
    /// and returns at once (see
    /// [`Commands::change_stream`](tobii_usb::engine::Commands::change_stream)),
    /// and dropping the answer does not take the change back. By default,
    /// for a stand-in, the change's own command (a 1220 or a 1230,
    /// [`StreamChange::device_command`]) runs through [`Self::run`] before
    /// this returns, its answer ready.
    #[must_use = "the answer comes through it; dropping it does not take the change back"]
    fn change_stream(&self, change: StreamChange, timeout: Duration) -> StreamAnswer {
        let DeviceCommand {
            cmd,
            payload,
            timeout,
        } = change.device_command(timeout);
        StreamAnswer::answered(self.run(cmd, payload, timeout))
    }
}

impl DeviceCommands for Commands {
    fn run(
        &self,
        cmd: u32,
        payload: Vec<u8>,
        timeout: Duration,
    ) -> Result<CommandResponse, CommandError> {
        Commands::run(
            self,
            DeviceCommand {
                cmd,
                payload,
                timeout,
            },
        )
    }

    // Not the default: the engine's `run` refuses a 1220 or a 1230
    // (`NotSupported`) and sends nothing, so through it no start or stop
    // would reach the tracker.
    fn change_stream(&self, change: StreamChange, timeout: Duration) -> StreamAnswer {
        Commands::change_stream(self, change, timeout)
    }
}

/// Run a command and reduce the outcome to its payload or a Stream Engine
/// status code. An answer succeeds only with the OK status word and no TTP
/// error; anything else is the device refusing the command.
pub(crate) fn run(
    device: &dyn DeviceCommands,
    cmd: u32,
    payload: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, u8> {
    run_with(device, cmd, payload, timeout, false)
}

/// [`run`] for a request that needs the device's calibration session
/// (collect, discard, clear, compute): refused for a bad state, it is
/// `CALIBRATION_NOT_STARTED`, the device having no session.
pub(crate) fn run_in_session(
    device: &dyn DeviceCommands,
    cmd: u32,
    payload: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, u8> {
    run_with(device, cmd, payload, timeout, true)
}

/// [`run`] and [`run_in_session`]: a refusal maps through [`refusal`] with
/// `in_session`.
fn run_with(
    device: &dyn DeviceCommands,
    cmd: u32,
    payload: Vec<u8>,
    timeout: Duration,
    in_session: bool,
) -> Result<Vec<u8>, u8> {
    match device.run(cmd, payload, timeout) {
        Ok(r) if r.status == RESPONSE_STATUS_OK && r.error == ttp_error::NONE => Ok(r.payload),
        Ok(r) => {
            tracing::warn!(
                cmd,
                status = r.status,
                error = format_args!("{:#x}", r.error),
                "device refused the command"
            );
            Err(refusal(r.error, in_session))
        }
        Err(CommandError::Timeout) => Err(status::TIMED_OUT),
        Err(e) => {
            tracing::warn!(cmd, error = %e, "device command failed");
            Err(status::CONNECTION_FAILED)
        }
    }
}

/// The status a refusal carrying TTP `error` gives. It follows the Windows
/// engine's mapping for an invalid parameter and for a failed operation; the
/// codes the engine gives `INTERNAL` (not supported, unknown) come back as
/// `OPERATION_FAILED`, since the daemon protocol has no `INTERNAL`. A bad state
/// means no session only to a request that needs one (`in_session`); anywhere
/// else (a start, a stop, a read or write of the calibration) it is a failed
/// operation. The request decides, not the command id: a start sends 1060 as
/// a clear does.
fn refusal(error: u32, in_session: bool) -> u8 {
    match error {
        // A command the device does not implement, which the DLL reports as
        // not supported (see the constant).
        ttp_error::NOT_SUPPORTED => status::NOT_SUPPORTED,
        ttp_error::INVALID_PARAMETER => status::INVALID_PARAMETER,
        ttp_error::BAD_STATE if in_session => status::CALIBRATION_NOT_STARTED,
        _ => status::OPERATION_FAILED,
    }
}

/// What came of a catalogue stream's `change`, `answer` being the tracker's
/// answer or why none came: the tracker result an `OK` reply carries
/// ([`tracker_error`]), or the status of a change the engine does not make.
/// An answer is judged as the DLL judges one, by its error word alone,
/// whatever its status word, which the DLL never reads (see
/// [`tracker_result`]). No answer in time is `FIRMWARE_NO_RESPONSE`, and the
/// tracker lost while the change was out (the engine stopped, or the open
/// it was written in ended, or its write failed) `CONNECTION_FAILED`. A
/// change the engine does not make, of a stream its catalogue does not
/// list, is `NOT_SUPPORTED` (`Err`): it wrote nothing. A refusal is logged,
/// at warn but for the one the ET5 gives every start of image collection
/// (see [`note_refusal`]).
pub(crate) fn stream_result(
    change: StreamChange,
    answer: Result<CommandResponse, CommandError>,
) -> Result<u32, u8> {
    match answer {
        Ok(response) => {
            if response.error != ttp_error::NONE {
                note_refusal(change, &response, &KNOWN_REFUSAL_LOGGED);
            }
            Ok(tracker_result(response.error))
        }
        Err(CommandError::Timeout) => Ok(tracker_error::FIRMWARE_NO_RESPONSE),
        Err(CommandError::NotSupported) => Err(status::NOT_SUPPORTED),
        Err(e) => {
            warn!(
                cmd = command_of(change),
                stream = %format_args!("{:#x}", change.stream_id()),
                error = %e,
                "a catalogue stream's start or stop failed"
            );
            Ok(tracker_error::CONNECTION_FAILED)
        }
    }
}

/// The tracker result ([`tracker_error`]) the DLL gives an answer carrying
/// TTP `error`: its response reader maps the error word to a code of its own
/// (0x18017b4c0), and a table maps that to the result (0x18018fc60). Neither
/// reads the status word.
#[must_use]
fn tracker_result(error: u32) -> u32 {
    match error {
        ttp_error::NONE => tracker_error::NO_ERROR,
        0x2000_0500 | 0x2000_0501 | 0x2000_0504 | 0x2000_0506 => tracker_error::NOT_SUPPORTED,
        ttp_error::INVALID_PARAMETER => tracker_error::INVALID_PARAMETER,
        0x2000_0407 | 0x2000_0502 | 0x2000_0511..=0x2000_0514 => tracker_error::OPERATION_FAILED,
        ttp_error::BAD_STATE => tracker_error::BAD_STATE,
        0x2000_0516 => tracker_error::TOO_MANY_SUBSCRIBERS,
        // `ttp_error::UNAUTHORIZED` (0x20000505) among them.
        _ => tracker_error::INTERNAL,
    }
}

/// The command a catalogue stream's `change` sends.
const fn command_of(change: StreamChange) -> u32 {
    match change {
        StreamChange::Start(_) => cmd::STREAM_START,
        StreamChange::Stop(_) => cmd::STREAM_STOP,
    }
}

/// The daemon's refusal of a start of image collection was logged at info
/// (see [`note_refusal`]).
static KNOWN_REFUSAL_LOGGED: AtomicBool = AtomicBool::new(false);

/// Log the tracker's refusal (`response`) of a catalogue stream's `change`,
/// and say at which level: at warn, but for the refusal the ET5 gives every
/// start of image collection on Linux (0x508, 0x20000505, taken to mean the
/// host lacks a realm's authorization), which an application may meet at
/// each of its subscribes. That one is logged at info the first time
/// `logged` sees it (the daemon's refusals share [`KNOWN_REFUSAL_LOGGED`]),
/// and at debug after.
fn note_refusal(change: StreamChange, response: &CommandResponse, logged: &AtomicBool) -> Level {
    let known = change == StreamChange::Start(STREAM_ID_IMAGE_COLLECTION)
        && response.error == ttp_error::UNAUTHORIZED;
    if !known {
        warn!(
            cmd = command_of(change),
            stream = %format_args!("{:#x}", change.stream_id()),
            status = response.status,
            error = format_args!("{:#x}", response.error),
            "the tracker refused a catalogue stream's start or stop"
        );
        return Level::WARN;
    }
    // Relaxed: a pure flag, no data is published with it.
    if logged.swap(true, Ordering::Relaxed) {
        debug!("the tracker refused to start image collection (0x20000505)");
        return Level::DEBUG;
    }
    info!(
        "the tracker refused to start image collection (0x20000505), as the ET5 does on Linux; \
         an application's subscription to it fails (logged once, then at debug)"
    );
    Level::INFO
}

#[cfg(test)]
mod tests {
    use super::*;
    use tobii_proto::calibration;
    use tobii_proto::protocol::cmd::{DEVICE_PAUSE, DISPLAY_AREA_SET};

    /// Answers every command with one fixed outcome.
    struct Answering(Result<CommandResponse, CommandError>);

    impl DeviceCommands for Answering {
        fn run(
            &self,
            _cmd: u32,
            _payload: Vec<u8>,
            _timeout: Duration,
        ) -> Result<CommandResponse, CommandError> {
            self.0.clone()
        }
    }

    /// A device answering with `status` and `error`.
    fn answer(status: u32, error: u32) -> Answering {
        Answering(Ok(CommandResponse {
            status,
            error,
            payload: vec![7],
        }))
    }

    fn outcome(cmd: u32, status: u32, error: u32) -> Result<Vec<u8>, u8> {
        run(
            &answer(status, error),
            cmd,
            Vec::new(),
            Duration::from_secs(1),
        )
    }

    fn outcome_in_session(cmd: u32, status: u32, error: u32) -> Result<Vec<u8>, u8> {
        run_in_session(
            &answer(status, error),
            cmd,
            Vec::new(),
            Duration::from_secs(1),
        )
    }

    #[test]
    fn a_command_the_device_does_not_implement_is_not_supported() {
        // What the ET5 answers to 2010, 2030, 3040 and 3050 (probe
        // 2026-10-21), whatever the command and in a session or not.
        for cmd in [2010, 2030, 3040, 3050] {
            assert_eq!(
                outcome(cmd, 1, ttp_error::NOT_SUPPORTED),
                Err(status::NOT_SUPPORTED),
                "cmd {cmd}"
            );
        }
        assert_eq!(
            outcome_in_session(calibration::cmd::COLLECT_2D, 1, ttp_error::NOT_SUPPORTED),
            Err(status::NOT_SUPPORTED)
        );
    }

    #[test]
    fn only_an_ok_status_without_an_error_succeeds() {
        assert_eq!(outcome(DISPLAY_AREA_SET, 1, 0), Ok(vec![7]));
        assert_eq!(
            outcome_in_session(calibration::cmd::COLLECT_2D, 1, 0),
            Ok(vec![7])
        );
        assert_eq!(
            outcome(DISPLAY_AREA_SET, 2, 0),
            Err(status::OPERATION_FAILED)
        );
        assert_eq!(
            outcome(DISPLAY_AREA_SET, 1, 0x2000_0407),
            Err(status::OPERATION_FAILED)
        );
    }

    #[test]
    fn a_ttp_error_maps_to_a_stream_engine_status() {
        for (status_word, error, expected) in [
            (1, ttp_error::INVALID_PARAMETER, status::INVALID_PARAMETER),
            (1, 0x2000_0407, status::OPERATION_FAILED),
            // Not supported: INTERNAL in the Windows engine.
            (1, 0x2000_0500, status::OPERATION_FAILED),
            (2, ttp_error::NONE, status::OPERATION_FAILED),
        ] {
            assert_eq!(
                outcome(DISPLAY_AREA_SET, status_word, error),
                Err(expected),
                "status {status_word}, error {error:#x}"
            );
            // A request that needs the session maps these as any other does.
            assert_eq!(
                outcome_in_session(calibration::cmd::COLLECT_2D, status_word, error),
                Err(expected),
                "in a session: status {status_word}, error {error:#x}"
            );
        }
    }

    #[test]
    fn a_bad_state_means_no_session_only_to_a_request_that_needs_one() {
        assert_eq!(
            outcome_in_session(calibration::cmd::COLLECT_2D, 1, ttp_error::BAD_STATE),
            Err(status::CALIBRATION_NOT_STARTED)
        );
        // The command id does not decide: a collect run as any other command
        // is a failed operation too.
        for cmd in [
            calibration::cmd::START,
            calibration::cmd::STOP,
            calibration::cmd::COLLECT_2D,
            calibration::cmd::READ,
            calibration::cmd::WRITE,
            1000,
            DISPLAY_AREA_SET,
            DEVICE_PAUSE,
        ] {
            assert_eq!(
                outcome(cmd, 1, ttp_error::BAD_STATE),
                Err(status::OPERATION_FAILED),
                "cmd {cmd}"
            );
        }
    }

    #[test]
    fn no_answer_is_a_timeout_or_a_lost_connection() {
        let ask = |e: CommandError| {
            run(
                &Answering(Err(e)),
                DISPLAY_AREA_SET,
                Vec::new(),
                Duration::from_secs(1),
            )
        };
        assert_eq!(ask(CommandError::Timeout), Err(status::TIMED_OUT));
        assert_eq!(
            ask(CommandError::EngineGone),
            Err(status::CONNECTION_FAILED)
        );
    }

    /// A catalogue stream's start or stop answered with TTP `error`.
    fn answered(error: u32) -> Result<CommandResponse, CommandError> {
        Ok(CommandResponse {
            error,
            ..CommandResponse::ok(Vec::new())
        })
    }

    /// An answer to a catalogue stream's start or stop is the tracker
    /// result the DLL gives its error word (0x18017b4c0, then 0x18018fc60),
    /// INTERNAL for any error its table does not name; the old requests'
    /// statuses are not touched.
    #[test]
    fn a_catalogue_stream_answer_is_the_trackers_result_as_the_dll_gives_it() {
        use tobii_ipc::request::tracker_error::*;
        let start = StreamChange::Start(0x1770);
        for (error, result) in [
            (0, NO_ERROR),
            (0x2000_0500, NOT_SUPPORTED),
            (0x2000_0501, NOT_SUPPORTED),
            (0x2000_0502, OPERATION_FAILED),
            (0x2000_0503, INTERNAL),
            (0x2000_0504, NOT_SUPPORTED),
            (0x2000_0505, INTERNAL),
            (0x2000_0506, NOT_SUPPORTED),
            (0x2000_0507, INTERNAL),
            (0x2000_0508, BAD_STATE),
            (0x2000_0509, INVALID_PARAMETER),
            (0x2000_050a, INTERNAL),
            (0x2000_0510, INTERNAL),
            (0x2000_0511, OPERATION_FAILED),
            (0x2000_0512, OPERATION_FAILED),
            (0x2000_0513, OPERATION_FAILED),
            (0x2000_0514, OPERATION_FAILED),
            (0x2000_0515, INTERNAL),
            (0x2000_0516, TOO_MANY_SUBSCRIBERS),
            (0x2000_0517, INTERNAL),
            (0x2000_0402, INTERNAL),
            (0x2000_0407, OPERATION_FAILED),
            (0x2000_040a, INTERNAL),
            (0x2000_04ff, INTERNAL),
            (1, INTERNAL),
            (u32::MAX, INTERNAL),
        ] {
            assert_eq!(
                stream_result(start, answered(error)),
                Ok(result),
                "error {error:#x}"
            );
        }
        // A display-area refusal of these errors is what it was.
        assert_eq!(
            outcome(DISPLAY_AREA_SET, 1, 0x2000_0505),
            Err(status::OPERATION_FAILED)
        );
    }

    /// The status word judges nothing: an answer without an error is the
    /// command taken, whatever its status word, as the DLL never reads it;
    /// one with an error is refused, whatever its status word.
    #[test]
    fn a_catalogue_stream_answer_is_judged_by_its_error_word_alone() {
        use tobii_ipc::request::tracker_error::{INVALID_PARAMETER, NO_ERROR};
        for status_word in [0, 1, 2, u32::MAX] {
            for change in [StreamChange::Start(0x508), StreamChange::Stop(0x508)] {
                let answer = |error| {
                    Ok(CommandResponse {
                        status: status_word,
                        error,
                        payload: Vec::new(),
                    })
                };
                assert_eq!(
                    stream_result(change, answer(ttp_error::NONE)),
                    Ok(NO_ERROR),
                    "{change:?}, status {status_word}"
                );
                assert_eq!(
                    stream_result(change, answer(ttp_error::INVALID_PARAMETER)),
                    Ok(INVALID_PARAMETER),
                    "{change:?}, status {status_word}"
                );
            }
        }
    }

    /// No answer in time is the DLL's `FIRMWARE_NO_RESPONSE`, and the
    /// tracker lost while the change was out `CONNECTION_FAILED`; a change
    /// the engine does not make is the request's `NOT_SUPPORTED`.
    #[test]
    fn a_catalogue_stream_change_without_an_answer_has_a_result_too() {
        use tobii_ipc::request::tracker_error::{CONNECTION_FAILED, FIRMWARE_NO_RESPONSE};
        let stop = StreamChange::Stop(0x1770);
        for (error, result) in [
            (CommandError::Timeout, Ok(FIRMWARE_NO_RESPONSE)),
            (CommandError::EngineGone, Ok(CONNECTION_FAILED)),
            (CommandError::Usb("pipe".into()), Ok(CONNECTION_FAILED)),
            (CommandError::NotSupported, Err(status::NOT_SUPPORTED)),
        ] {
            assert_eq!(stream_result(stop, Err(error.clone())), result, "{error}");
        }
    }

    /// The ET5 refuses every start of image collection: logged at info the
    /// first time, at debug after. Any other refusal, of image collection's
    /// stop, of the custom stream's start or with another error, is warned
    /// of every time.
    #[test]
    fn only_the_refusal_of_image_collection_the_et5_gives_is_logged_quietly() {
        let logged = AtomicBool::new(false);
        let refused = |error| CommandResponse {
            error,
            ..CommandResponse::ok(Vec::new())
        };
        let level = |change, error| note_refusal(change, &refused(error), &logged);
        let start = StreamChange::Start(STREAM_ID_IMAGE_COLLECTION);

        assert_eq!(level(start, ttp_error::UNAUTHORIZED), Level::INFO);
        assert_eq!(level(start, ttp_error::UNAUTHORIZED), Level::DEBUG);
        assert_eq!(level(start, ttp_error::UNAUTHORIZED), Level::DEBUG);
        for (change, error) in [
            (start, ttp_error::INVALID_PARAMETER),
            (
                StreamChange::Stop(STREAM_ID_IMAGE_COLLECTION),
                ttp_error::UNAUTHORIZED,
            ),
            (StreamChange::Start(0x1770), ttp_error::UNAUTHORIZED),
        ] {
            assert_eq!(level(change, error), Level::WARN, "{change:?}, {error:#x}");
        }
        let fresh = AtomicBool::new(false);
        assert_eq!(
            note_refusal(start, &refused(ttp_error::INVALID_PARAMETER), &fresh),
            Level::WARN
        );
        assert!(!fresh.load(Ordering::Relaxed), "only that refusal counts");
    }

    /// Answers every command and logs it with its payload and deadline.
    #[derive(Default)]
    struct Logging(std::sync::Mutex<Vec<(u32, Vec<u8>, Duration)>>);

    impl DeviceCommands for Logging {
        fn run(
            &self,
            cmd: u32,
            payload: Vec<u8>,
            timeout: Duration,
        ) -> Result<CommandResponse, CommandError> {
            self.0.lock().expect("log").push((cmd, payload, timeout));
            Ok(CommandResponse::ok(vec![7]))
        }
    }

    /// A device that does not make a stream change itself (a stand-in) runs
    /// the change's command, the 1220 or 1230 the engine would send, with
    /// its deadline, and the answer is there at once.
    #[test]
    fn a_stream_change_runs_its_command_on_a_device_that_does_not_make_it() {
        use tobii_proto::protocol::{stream_id_payload, stream_start_payload};
        let device = Logging::default();
        let due = Duration::from_secs(3);

        let started = device
            .change_stream(StreamChange::Start(0x1770), due)
            .wait();
        let stopped = device.change_stream(StreamChange::Stop(0x508), due).wait();

        assert_eq!(started, Ok(CommandResponse::ok(vec![7])));
        assert_eq!(stopped, Ok(CommandResponse::ok(vec![7])));
        assert_eq!(
            *device.0.lock().expect("log"),
            [
                (cmd::STREAM_START, stream_start_payload(0x1770), due),
                (cmd::STREAM_STOP, stream_id_payload(0x508), due),
            ]
        );
    }

    /// The engine's handle queues a stream change on the engine; it does not
    /// run the change's command, as the trait's default would: the engine's
    /// `run` refuses a 1220 or a 1230, so every start would be the request's
    /// `NOT_SUPPORTED`, and no stop would be sent. A handle no engine serves
    /// tells the two apart: what it queues is answered `EngineGone`.
    #[test]
    fn the_engines_handle_queues_a_stream_change_whose_command_its_run_refuses() {
        let device: &dyn DeviceCommands = &Commands::unserved();
        let due = Duration::from_secs(3);
        for change in [
            StreamChange::Start(0x1770),
            StreamChange::Stop(0x1770),
            StreamChange::Start(STREAM_ID_IMAGE_COLLECTION),
            StreamChange::Stop(STREAM_ID_IMAGE_COLLECTION),
        ] {
            assert_eq!(
                device.change_stream(change, due).wait(),
                Err(CommandError::EngineGone),
                "{change:?}, queued"
            );
            let command = change.device_command(due);
            assert_eq!(
                device.run(command.cmd, command.payload, command.timeout),
                Err(CommandError::NotSupported),
                "{change:?}, its command run"
            );
        }
        assert_eq!(
            device.run(DISPLAY_AREA_SET, Vec::new(), due),
            Err(CommandError::EngineGone),
            "any other command run, queued"
        );
    }
}
