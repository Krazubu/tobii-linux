//! The state-changing exports of Stream Engine 4.1 that libtobii carries out
//! rather than refuses, and the answers that go with them.
//!
//! Each was decoded from `tobii_stream_engine.dll` 4.1.0.3 and then put to the
//! ET5 itself (probes of 2026-10-21; the reasoning, addresses and answers are
//! in `fixtures/analysis/state-commands-2026-10-21`). The ones the tracker does
//! not implement stay refusals, in [`crate::internal`]: remote wake (3040,
//! 3050) and reading the illumination mode (2010, 2030) are answered
//! `0x20000501` by the ET5, which the DLL turns into
//! `TOBII_ERROR_NOT_SUPPORTED` — exactly what those refusals already answer.

use crate::calibration::request;
use crate::device::Device;
use crate::status::Status;
use crate::timeouts;
use tobii_ipc::request::kind;

/// Put the tracker into power save.
///
/// The DLL's export (0x18014e0e0) sets PRP property 2, `POWER_SAVE_ACTIVE`,
/// which an ET5 lists as writable, so its own module sends command 3000; the
/// ET5 carries it out. Power save does not stop the sensor: gaze keeps coming
/// at full rate. Like the pause, it is one state for the tracker, shared by
/// every client: the last call wins and any client may end it; it ends when
/// the client that put the tracker into it disconnects, where the DLL, living
/// in one process, has nothing to do.
///
/// The call does not change `TOBII_STATE_POWER_SAVE_ACTIVE` by itself, as in
/// the DLL, whose setter writes no cache: the tracker's notification 3020
/// does, which also reaches subscribers as
/// `TOBII_NOTIFICATION_TYPE_POWER_SAVE_STATE_CHANGED`. The ET5 sends it at
/// once. A daemon too old to know the request answers
/// `TOBII_ERROR_NOT_SUPPORTED`.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_power_save_activate(device: *mut Device) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { request(device, kind::POWER_SAVE, &[1], timeouts::POWER_SAVE) }
}

/// Take the tracker out of power save, whichever client put it there: command
/// 3010, as the DLL's export (0x18014e030) sends for PRP property 2. The
/// tracker announces the change with notification 3030. As
/// `tobii_power_save_activate` otherwise.
///
/// # Safety
/// As `tobii_power_save_activate`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_power_save_deactivate(device: *mut Device) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { request(device, kind::POWER_SAVE, &[0], timeouts::POWER_SAVE) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tobii_device_destroy;
    use crate::status::{TOBII_ERROR_NO_ERROR, TOBII_ERROR_NOT_SUPPORTED};
    use std::sync::{Arc, Mutex};
    use tobii_ipc::request::{self, status};

    /// Requests a fake daemon was sent, as `(kind, payload)`.
    type Requests = Arc<Mutex<Vec<(u8, Vec<u8>)>>>;

    /// A device on a fake daemon answering every request `answer`, and the
    /// requests it was sent.
    fn daemon(answer: u8) -> (*mut Device, Requests) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let connect = crate::device::tests::fake_daemon(move |body| {
            let req = request::decode_request(body).expect("request");
            log.lock()
                .expect("log")
                .push((req.kind, req.payload.to_vec()));
            vec![tobii_ipc::encode_reply(req.id, answer, &[])]
        });
        let d = Box::into_raw(Box::new(Device::new(connect, 1, 1).expect("device")));
        (d, seen)
    }

    #[test]
    fn power_save_is_entered_and_left_with_one_request_each() {
        let (d, seen) = daemon(status::OK);
        // SAFETY: `d` is live and destroyed once.
        unsafe {
            assert_eq!(tobii_power_save_activate(d), TOBII_ERROR_NO_ERROR);
            assert_eq!(tobii_power_save_deactivate(d), TOBII_ERROR_NO_ERROR);
            assert_eq!(tobii_device_destroy(d), 0);
        }
        assert_eq!(
            *seen.lock().expect("log"),
            [(kind::POWER_SAVE, vec![1]), (kind::POWER_SAVE, vec![0])]
        );
    }

    #[test]
    fn a_daemon_that_predates_power_save_answers_not_supported() {
        let (d, _) = daemon(status::NOT_SUPPORTED);
        // SAFETY: `d` is live and destroyed once.
        unsafe {
            assert_eq!(tobii_power_save_activate(d), TOBII_ERROR_NOT_SUPPORTED);
            assert_eq!(tobii_power_save_deactivate(d), TOBII_ERROR_NOT_SUPPORTED);
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    #[test]
    fn a_null_device_is_an_invalid_parameter() {
        // SAFETY: a null device is allowed and refused.
        unsafe {
            assert_eq!(
                tobii_power_save_activate(std::ptr::null_mut()),
                crate::status::TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_power_save_deactivate(std::ptr::null_mut()),
                crate::status::TOBII_ERROR_INVALID_PARAMETER
            );
        }
    }
}
