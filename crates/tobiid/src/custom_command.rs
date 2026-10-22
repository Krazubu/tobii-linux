//! An application's own command to the tracker: command 5000, behind
//! `tobii_send_custom_command`.
//!
//! The DLL hard-codes the command id (the immediate 0x1388 at 0x180194d6a in
//! `tracker_send_custom_command`, 0x1801948d0), so the number the application
//! passes never reaches the tracker: its module refuses anything but zero
//! before the tracker is touched (`platmod_legacy_ttp.cpp` line 3865), which
//! libtobii does too. What arrives here is the blob, which goes out as the
//! command's one bytes TLV, and the tracker's own blob comes back.
//!
//! **The ET5 knows the command.** Probed 2026-10-22, a four-zero-byte blob
//! was answered `0x20000509` (invalid parameter) with no payload — not the
//! `0x20000501` it gives a command it does not have (as it does for 2010,
//! 2030, 3040 and 3050). So this is a real channel; what a valid blob looks
//! like is the application's business, and nothing in the DLL or any capture
//! says. Nothing about tracker state changed in that probe: the 1490 before
//! and after matched and gaze kept its rate.
//!
//! The reply carries the DLL's tracker result rather than a reduced status,
//! because libtobii has to map it through the DLL's own three tables, whose
//! composition differs from every other request's (see
//! `tobii-ffi`'s `custom_command`).

use std::sync::Mutex;
use std::time::Duration;

use tobii_ipc::deadline;
use tobii_ipc::request::{CUSTOM_COMMAND_MAX, encode_custom_command_answer, status, tracker_error};
use tobii_proto::facts::{custom_command_answer, custom_command_payload};
use tobii_proto::protocol::cmd;
use tracing::warn;

use crate::daemon::{State, lock_state, look_for_tracker};
use crate::device::command_result;
use crate::requests::Reply;

/// How long the device may take to answer command 5000.
const CUSTOM_COMMAND_TIMEOUT: Duration = deadline::CUSTOM_COMMAND;

/// Send `payload` to the tracker as command 5000 and answer with the tracker
/// result and its blob.
///
/// A blob larger than [`CUSTOM_COMMAND_MAX`] is refused without being sent:
/// the DLL bounds neither end, but its own reply path copies a
/// device-supplied length into a block allocated without a size, so this port
/// bounds both. An answer that is not the one bytes TLV the DLL's schema
/// demands is `CONNECTION_FAILED`, as the DLL answers an answer it cannot
/// read.
pub(crate) fn handle(state: &Mutex<State>, client: u64, payload: &[u8]) -> Reply {
    if payload.len() > CUSTOM_COMMAND_MAX {
        warn!(
            client,
            len = payload.len(),
            max = CUSTOM_COMMAND_MAX,
            "the application's command carries too large a blob; not sent"
        );
        return Reply::err(status::INVALID_PARAMETER);
    }
    let on_bus = look_for_tracker(state);
    let Some(device) = lock_state(state).device_for(client, on_bus) else {
        return Reply::err(status::CONNECTION_FAILED);
    };
    let sent = device.run(
        cmd::CUSTOM_COMMAND,
        custom_command_payload(payload),
        CUSTOM_COMMAND_TIMEOUT,
    );
    let (result, answer) = match command_result(cmd::CUSTOM_COMMAND, sent) {
        Ok(got) => got,
        Err(code) => return Reply::err(code),
    };
    if result != tracker_error::NO_ERROR {
        return Reply::ok(encode_custom_command_answer(result, &[]));
    }
    // Only a successful answer carries a blob, and only in the shape the
    // DLL's response schema takes: one bytes TLV, nothing else.
    let Some(blob) = custom_command_answer(&answer) else {
        warn!(
            client,
            len = answer.len(),
            "the tracker's answer to the application's command is not one blob; refused"
        );
        return Reply::err(status::CONNECTION_FAILED);
    };
    if blob.len() > CUSTOM_COMMAND_MAX {
        warn!(
            client,
            len = blob.len(),
            max = CUSTOM_COMMAND_MAX,
            "the tracker answered the application's command with too large a blob; refused"
        );
        return Reply::err(status::CONNECTION_FAILED);
    }
    Reply::ok(encode_custom_command_answer(result, &blob))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::tests::state_with_client;
    use crate::device::DeviceCommands;
    use std::sync::Arc;
    use tobii_ipc::request::{Request, decode_custom_command_answer, kind};
    use tobii_proto::protocol::ttp_error;
    use tobii_usb::engine::{CommandError, CommandResponse};

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

    /// A tracker answer carrying `error` and `payload`.
    fn answered(error: u32, payload: Vec<u8>) -> Result<CommandResponse, CommandError> {
        Ok(CommandResponse {
            status: tobii_proto::protocol::RESPONSE_STATUS_OK,
            error,
            payload,
        })
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
                kind: kind::CUSTOM_COMMAND,
                payload,
            },
        )
    }

    #[test]
    fn a_successful_answer_carries_the_tracker_s_blob_and_sends_only_5000() {
        let (state, device) = setup(answered(
            ttp_error::NONE,
            custom_command_payload(&[9, 8, 7]),
        ));
        let reply = ask(&state, &[1, 2, 3]);
        assert_eq!(reply.status, status::OK);
        assert_eq!(
            decode_custom_command_answer(&reply.payload),
            Some((tracker_error::NO_ERROR, &[9u8, 8, 7][..]))
        );
        // The id the DLL hard-codes, and the application's blob as the one
        // bytes TLV it writes.
        assert_eq!(
            *device.sent.lock().expect("sent"),
            vec![(cmd::CUSTOM_COMMAND, custom_command_payload(&[1, 2, 3]))]
        );
        assert_eq!(
            *device.deadlines.lock().expect("deadlines"),
            vec![CUSTOM_COMMAND_TIMEOUT]
        );
    }

    #[test]
    fn the_et5_s_probed_refusal_comes_back_as_its_tracker_result() {
        // What the tracker really answered on 2026-10-22: 0x20000509 with no
        // payload. libtobii turns that result into
        // TOBII_ERROR_INVALID_PARAMETER.
        let (state, _d) = setup(answered(ttp_error::INVALID_PARAMETER, Vec::new()));
        let reply = ask(&state, &[0, 0, 0, 0]);
        assert_eq!(reply.status, status::OK);
        assert_eq!(
            decode_custom_command_answer(&reply.payload),
            Some((tracker_error::INVALID_PARAMETER, &[][..])),
            "the result, and no blob"
        );
    }

    #[test]
    fn a_refusal_never_carries_a_blob_even_if_the_tracker_sent_one() {
        let (state, _d) = setup(answered(
            ttp_error::NOT_SUPPORTED,
            custom_command_payload(&[1, 2, 3]),
        ));
        let reply = ask(&state, &[0]);
        assert_eq!(
            decode_custom_command_answer(&reply.payload),
            Some((tracker_error::NOT_SUPPORTED, &[][..]))
        );
    }

    #[test]
    fn an_answer_that_is_not_one_blob_is_refused_as_unreadable() {
        // A success whose payload is not the one bytes TLV the DLL's schema
        // demands: no TLV at all, or two.
        let (state, _d) = setup(answered(ttp_error::NONE, Vec::new()));
        assert_eq!(ask(&state, &[0]).status, status::CONNECTION_FAILED);
        let mut two = tobii_proto::tlv::TlvWriter::new();
        two.bytes(&[1]).bytes(&[2]);
        let (state, _d) = setup(answered(ttp_error::NONE, two.finish()));
        assert_eq!(ask(&state, &[0]).status, status::CONNECTION_FAILED);
    }

    #[test]
    fn a_blob_past_the_bound_is_refused_in_both_directions() {
        // Out: refused before anything is sent.
        let (state, device) = setup(answered(ttp_error::NONE, Vec::new()));
        let reply = ask(&state, &vec![0; CUSTOM_COMMAND_MAX + 1]);
        assert_eq!(reply.status, status::INVALID_PARAMETER);
        assert!(
            device.sent.lock().expect("sent").is_empty(),
            "nothing is sent for an over-large blob"
        );

        // Back: the bound the DLL does not have on its own reply path, where
        // it copies a device-supplied length into an unsized block.
        let (state, _d) = setup(answered(
            ttp_error::NONE,
            custom_command_payload(&vec![0; CUSTOM_COMMAND_MAX + 1]),
        ));
        assert_eq!(ask(&state, &[0]).status, status::CONNECTION_FAILED);
    }

    #[test]
    fn a_tracker_that_does_not_answer_in_time_is_the_dll_s_no_response() {
        let (state, _d) = setup(Err(CommandError::Timeout));
        let reply = ask(&state, &[0]);
        assert_eq!(reply.status, status::OK);
        assert_eq!(
            decode_custom_command_answer(&reply.payload),
            Some((tracker_error::FIRMWARE_NO_RESPONSE, &[][..])),
            "which libtobii maps to CONNECTION_FAILED_DRIVER, as the DLL does"
        );
    }
}
