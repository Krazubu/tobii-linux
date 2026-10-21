//! The tracker's extensions: commands 1810 (every extension) and 1820 (the
//! ones switched on), as the DLL's `tracker_enumerate_extensions` and
//! `tracker_enumerate_enabled_extensions` (0x18019f5b0, 0x18019d050) send
//! them. Neither carries a payload.
//!
//! The DLL sends one on every call and keeps no cache, so the daemon asks the
//! tracker on every request too. The ET5 answers both with an empty list
//! (probe 2026-10-21): it has no extensions at all, which is also why its two
//! answers are byte-identical. Since no entry was ever on the wire its layout
//! is not pinned, and a list with entries is refused rather than guessed at
//! (see [`parse_extensions`]); so is an answer that is not such a list, as the
//! DLL answers `TOBII_ERROR_CONNECTION_FAILED` to one it cannot read.
//!
//! Enabling an extension (1800) is not offered: nothing in the DLL can switch
//! one off again, and libtobii answers it as the DLL does for a consumer ET5.

use std::sync::Mutex;
use std::time::Duration;

use tobii_ipc::deadline;
use tobii_ipc::request::{EXTENSIONS_ALL, EXTENSIONS_ENABLED, encode_u32, status};
use tobii_proto::facts::parse_extensions;
use tobii_proto::protocol::cmd;
use tracing::warn;

use crate::daemon::{State, lock_state, look_for_tracker};
use crate::device::run;
use crate::requests::Reply;

/// How long the device may take to answer 1810 or 1820.
const EXTENSIONS_TIMEOUT: Duration = deadline::EXTENSIONS;

/// Answer a request for the extensions from `client`: payload
/// [`EXTENSIONS_ALL`] or [`EXTENSIONS_ENABLED`]; reply `u32`, how many the
/// tracker listed.
pub(crate) fn handle(state: &Mutex<State>, client: u64, payload: &[u8]) -> Reply {
    let command = match payload {
        [EXTENSIONS_ALL] => cmd::EXTENSIONS_ENUMERATE,
        [EXTENSIONS_ENABLED] => cmd::EXTENSIONS_ENUMERATE_ENABLED,
        _ => return Reply::err(status::INVALID_PARAMETER),
    };
    let on_bus = look_for_tracker(state);
    let Some(device) = lock_state(state).device_for(client, on_bus) else {
        return Reply::err(status::CONNECTION_FAILED);
    };
    match run(device.as_ref(), command, Vec::new(), EXTENSIONS_TIMEOUT) {
        Ok(answer) => match parse_extensions(&answer) {
            Some(ids) => Reply::ok(encode_u32(u32::try_from(ids.len()).unwrap_or(u32::MAX))),
            None => {
                warn!(
                    command,
                    len = answer.len(),
                    "extension answer with entries, or not a list at all; refused"
                );
                Reply::err(status::CONNECTION_FAILED)
            }
        },
        Err(code) => Reply::err(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::tests::state_with_client;
    use crate::device::DeviceCommands;
    use std::sync::Arc;
    use tobii_ipc::request::{Request, kind};
    use tobii_proto::protocol::ttp_error;
    use tobii_usb::engine::{CommandError, CommandResponse};

    /// The payload the ET5 answered 1810 and 1820 with on 2026-10-21: an
    /// empty list (header `0x0001_0100`, count 1 - 1 = 0) of element type 9000.
    const ET5_PAYLOAD: [u8; 20] = [
        0x00, 0x00, 0x05, 0, 0, 0, 4, 0x00, 0x01, 0x01, 0x00, 0x02, 0, 0, 0, 4, 0x00, 0x00, 0x23,
        0x28,
    ];

    /// Records every command and answers each with `answer`.
    struct Recording {
        sent: Mutex<Vec<(u32, Vec<u8>)>>,
        deadlines: Mutex<Vec<Duration>>,
        answer: Mutex<Result<CommandResponse, CommandError>>,
    }

    impl DeviceCommands for Recording {
        fn run(
            &self,
            cmd: u32,
            payload: Vec<u8>,
            timeout: Duration,
        ) -> Result<CommandResponse, CommandError> {
            self.sent.lock().expect("sent").push((cmd, payload));
            self.deadlines.lock().expect("deadlines").push(timeout);
            self.answer.lock().expect("answer").clone()
        }
    }

    fn setup(answer: Result<CommandResponse, CommandError>) -> (Mutex<State>, Arc<Recording>) {
        let device = Arc::new(Recording {
            sent: Mutex::new(Vec::new()),
            deadlines: Mutex::new(Vec::new()),
            answer: Mutex::new(answer),
        });
        let mut st = state_with_client(1);
        st.fake_device = Some(device.clone());
        (Mutex::new(st), device)
    }

    fn ask(state: &Mutex<State>, payload: &[u8]) -> Reply {
        crate::requests::handle(
            state,
            1,
            &Request {
                id: 1,
                kind: kind::EXTENSIONS,
                payload,
            },
        )
    }

    fn sent(device: &Recording) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut *device.sent.lock().expect("sent"))
    }

    #[test]
    fn the_et5s_answer_is_no_extensions_either_way() {
        let (state, device) = setup(Ok(CommandResponse::ok(ET5_PAYLOAD.to_vec())));
        assert_eq!(ask(&state, &[EXTENSIONS_ALL]), Reply::ok(encode_u32(0)));
        assert_eq!(sent(&device), [(1810, Vec::new())]);
        assert_eq!(ask(&state, &[EXTENSIONS_ENABLED]), Reply::ok(encode_u32(0)));
        assert_eq!(sent(&device), [(1820, Vec::new())]);
    }

    #[test]
    fn every_request_asks_the_tracker_as_the_dll_does() {
        let (state, device) = setup(Ok(CommandResponse::ok(ET5_PAYLOAD.to_vec())));
        ask(&state, &[EXTENSIONS_ALL]);
        ask(&state, &[EXTENSIONS_ALL]);
        assert_eq!(sent(&device).len(), 2, "no cache: one 1810 per request");
    }

    #[test]
    fn a_list_with_entries_or_no_list_is_refused() {
        // Count 1 (high half 2): an entry whose layout no capture shows.
        let mut with_entry = ET5_PAYLOAD.to_vec();
        with_entry[8] = 0x02;
        for payload in [with_entry, Vec::new(), vec![0x00, 0x00]] {
            let (state, _) = setup(Ok(CommandResponse::ok(payload.clone())));
            assert_eq!(
                ask(&state, &[EXTENSIONS_ALL]),
                Reply::err(status::CONNECTION_FAILED),
                "{payload:02x?}"
            );
        }
    }

    #[test]
    fn a_refusal_is_passed_on() {
        let (state, _) = setup(Ok(CommandResponse {
            status: 1,
            error: ttp_error::NOT_SUPPORTED,
            payload: Vec::new(),
        }));
        assert_eq!(
            ask(&state, &[EXTENSIONS_ALL]),
            Reply::err(status::NOT_SUPPORTED)
        );
    }

    #[test]
    fn a_selector_that_is_neither_is_refused_and_sends_nothing() {
        let (state, device) = setup(Ok(CommandResponse::ok(ET5_PAYLOAD.to_vec())));
        for payload in [&[][..], &[2], &[0, 0]] {
            assert_eq!(ask(&state, payload), Reply::err(status::INVALID_PARAMETER));
        }
        assert!(sent(&device).is_empty());
    }

    #[test]
    fn a_request_waits_on_the_device_within_its_budget() {
        let (state, device) = setup(Ok(CommandResponse::ok(ET5_PAYLOAD.to_vec())));
        assert_eq!(ask(&state, &[EXTENSIONS_ENABLED]), Reply::ok(encode_u32(0)));
        let sent = device.deadlines.lock().expect("deadlines").clone();
        let waited = deadline::commands(&sent);
        assert!(
            !sent.is_empty() && waited <= deadline::worst::EXTENSIONS,
            "{waited:?}"
        );
    }
}
