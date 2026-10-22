//! `tobii_send_custom_command`: an application's own command to the tracker.
//!
//! The one entry point of the seventeen that really needs the tracker, and
//! the one whose answer a probe had to settle (the derivation and every
//! address: `fixtures/analysis/unknown-protocols-2026-10-22/REPORT-A.md`
//! §2.4, the probe in `RESULTS-PROBE.md`, with `custom-command/REPORT.md`
//! beside them).
//!
//! What the DLL does with its `command` argument is the surprise: **it never
//! reaches the tracker**. `tracker_send_custom_command` (0x1801948d0) holds
//! the command id as an immediate, 0x1388 = 5000 (0x180194d6a), and the
//! number the application passes is only a gate — the module refuses
//! anything but zero before the tracker is touched (`platmod_legacy_ttp.cpp`
//! line 3865, 0x180168075). So a non-zero number is
//! `TOBII_ERROR_NOT_SUPPORTED` without a round trip, and a zero sends 5000
//! with the application's blob.
//!
//! **The ET5 knows command 5000**: probed 2026-10-22, it answered a
//! four-zero-byte blob `0x20000509` (invalid parameter) rather than the
//! `0x20000501` it gives a command it does not have. So this is a real
//! channel, and what a valid blob looks like is the application's business:
//! nothing in the DLL or any capture says. Nothing about the tracker's state
//! changed in that probe.

use std::ffi::c_void;

use crate::device::{Device, device_ref};
use crate::status::{
    Status, TOBII_ERROR_CONNECTION_FAILED_DRIVER, TOBII_ERROR_INTERNAL,
    TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NO_ERROR, TOBII_ERROR_NOT_SUPPORTED,
    TOBII_ERROR_OPERATION_FAILED,
};
use crate::timeouts;
use crate::types::DataReceiver;
use tobii_ipc::request::{CUSTOM_COMMAND_MAX, decode_custom_command_answer, kind, tracker_error};

/// The only command number the DLL lets through to the tracker: its module
/// refuses every other before the tracker is touched (0x180168075), the
/// number itself never reaching a request.
const PASSED_THROUGH: u32 = 0;

/// What `tobii_send_custom_command` answers for the [`tracker_error`] in
/// tobiid's reply: the DLL's three maps composed, its tracker module's (the
/// jump table at 0x180168438), its server's (0x18002e3f0, table 0x18002e444)
/// and its client's (0x18015c790, table 0x18015c808).
///
/// **Not the same composition as the custom stream's** (see
/// `internal::custom_stream_status`), though the two differ only where
/// tobiid never lands: this path sends `ALLOCATION_FAILED` to
/// `TOBII_ERROR_OPERATION_FAILED` where the stream's sends it to
/// `TOBII_ERROR_INTERNAL`, and `TOO_MANY_SUBSCRIBERS` the other way about.
/// The two tables really are different, so they get two functions.
///
/// So the ET5's probed refusal (`0x20000509`, tracker `INVALID_PARAMETER`) is
/// `TOBII_ERROR_INVALID_PARAMETER`; a tracker that does not answer in time,
/// one that is lost and one refusing `0x20000505` are all
/// `TOBII_ERROR_CONNECTION_FAILED_DRIVER`; and a command the tracker does not
/// have is `TOBII_ERROR_NOT_SUPPORTED`.
const fn custom_command_status(result: u32) -> Status {
    match result {
        tracker_error::NO_ERROR => TOBII_ERROR_NO_ERROR,
        tracker_error::INTERNAL
        | tracker_error::CONNECTION_FAILED
        | tracker_error::FIRMWARE_NO_RESPONSE => TOBII_ERROR_CONNECTION_FAILED_DRIVER,
        tracker_error::NOT_SUPPORTED => TOBII_ERROR_NOT_SUPPORTED,
        tracker_error::INVALID_PARAMETER => TOBII_ERROR_INVALID_PARAMETER,
        tracker_error::ALLOCATION_FAILED | tracker_error::OPERATION_FAILED => {
            TOBII_ERROR_OPERATION_FAILED
        }
        _ => TOBII_ERROR_INTERNAL,
    }
}

/// Send the application's own command to the tracker and hand its answer to
/// `receiver`.
///
/// The DLL's export (0x18014e8b0) checks, in this order: a null device
/// (`TOBII_ERROR_INVALID_PARAMETER`, unlogged), a null `data`
/// (`tobii_internal.cpp` line 177), a `size` of zero (line 178), a null
/// `receiver` (line 179), the callback flag (line 180), and then that the
/// device's PRP command list holds command 2 (line 182) — which an ET5's
/// always does, its tracker module listing the module command that maps to
/// it unconditionally, so that check cannot fire here. `command` is then a
/// pure gate: anything but zero is `TOBII_ERROR_NOT_SUPPORTED` from the
/// module, with nothing sent.
///
/// For zero it sends TTP command **5000** carrying `data` as one bytes TLV,
/// allowing the device 15 s, and the answer must be exactly one bytes TLV,
/// which is handed to `receiver` with `user_data`. The receiver runs **after**
/// the round trip, not under the callback flag — none of the functions behind
/// these commands sets it — and this crate's device lock is let go for the
/// call, so the receiver may call back into the device.
///
/// Where this differs from the DLL: both blobs are bounded at
/// [`CUSTOM_COMMAND_MAX`], where the DLL bounds neither and its own reply
/// path copies a device-supplied length into a block allocated without a size
/// (0x18002b0e8 allocates, 0x18002b0fd copies). A blob past the bound is
/// `TOBII_ERROR_INVALID_PARAMETER` going out, and coming back it is tobiid
/// that refuses it, which arrives here as `TOBII_ERROR_CONNECTION_FAILED` —
/// the answer the DLL gives one it cannot read — as does an answer that is
/// not the single bytes TLV its schema takes. A reply this crate cannot
/// decode at all, which tobiid should never send, is
/// `TOBII_ERROR_INTERNAL`, as everywhere here. An answer of no bytes is
/// passed on as the DLL passes it, with a null pointer and a zero size.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `data` must be null or valid
/// for reading `size` bytes; `receiver` must be null or sound to call with a
/// pointer valid during the call only, a size and `user_data`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_send_custom_command(
    device: *mut Device,
    command: u32,
    data: *const c_void,
    size: usize,
    receiver: Option<DataReceiver>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    if data.is_null() || size == 0 {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    let Some(receiver) = receiver else {
        return TOBII_ERROR_INVALID_PARAMETER;
    };
    if command != PASSED_THROUGH {
        // The module's refusal, which never reaches the tracker.
        tracing::trace!(command, "a custom command number the Stream Engine refuses");
        return TOBII_ERROR_NOT_SUPPORTED;
    }
    if size > CUSTOM_COMMAND_MAX {
        tracing::trace!(
            size,
            max = CUSTOM_COMMAND_MAX,
            "custom command blob too large"
        );
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    // SAFETY: `data` is non-null and the caller guarantees it is valid for
    // reading `size` bytes, which the bound above keeps to a sane length.
    let blob = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) };
    let answer = match d.request(kind::CUSTOM_COMMAND, blob, timeouts::CUSTOM_COMMAND) {
        Ok(payload) => payload,
        Err(status) => return status,
    };
    let Some((result, reply)) = decode_custom_command_answer(&answer) else {
        return d.malformed("custom command answer");
    };
    let status = custom_command_status(result);
    if status != TOBII_ERROR_NO_ERROR {
        return status;
    }
    // The DLL calls the receiver with the blob's pointer and size; a blob of
    // no bytes gives a null pointer, as its own empty-reply path would.
    let (ptr, len) = if reply.is_empty() {
        (std::ptr::null(), 0)
    } else {
        (reply.as_ptr().cast::<c_void>(), reply.len())
    };
    // SAFETY: the caller guarantees `receiver` is sound to call like this;
    // `reply` outlives the call.
    unsafe { receiver(ptr, len, user_data) };
    TOBII_ERROR_NO_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::{counting_device, destroy_untouched, fake_daemon};
    use crate::status::{TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_CONNECTION_FAILED};
    use std::ptr;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use tobii_ipc::request::{self, encode_custom_command_answer};

    /// What the receiver was handed.
    type Seen = Arc<Mutex<Vec<Option<Vec<u8>>>>>;

    thread_local! {
        static SEEN: std::cell::RefCell<Option<Seen>> = const { std::cell::RefCell::new(None) };
    }

    /// A receiver that records the blob it is given, `None` for a null one.
    unsafe extern "C" fn record(data: *const c_void, size: usize, _user: *mut c_void) {
        let blob = if data.is_null() {
            None
        } else {
            // SAFETY: the export hands a pointer valid for `size` bytes.
            Some(unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) }.to_vec())
        };
        SEEN.with_borrow(|s| {
            if let Some(seen) = s {
                seen.lock().expect("seen").push(blob);
            }
        });
    }

    /// A device whose daemon answers `CUSTOM_COMMAND` with `answer`, and
    /// records the request payloads it got.
    fn device_answering(answer: Vec<u8>) -> (*mut Device, Arc<Mutex<Vec<Vec<u8>>>>) {
        let sent: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let got = Arc::clone(&sent);
        let connect = fake_daemon(move |body| match body.first() {
            Some(&tobii_ipc::TAG_SUBSCRIBE) => vec![tobii_ipc::encode_subscribed(true)],
            Some(&tobii_ipc::TAG_REQUEST) => {
                let req = request::decode_request(body).expect("request");
                if req.kind == kind::CUSTOM_COMMAND {
                    got.lock().expect("sent").push(req.payload.to_vec());
                    vec![tobii_ipc::encode_reply(req.id, 0, &answer)]
                } else {
                    vec![tobii_ipc::encode_reply(
                        req.id,
                        request::status::NOT_SUPPORTED,
                        &[],
                    )]
                }
            }
            _ => vec![],
        });
        let d = Device::new(connect, 1, 1).expect("device");
        (Box::into_raw(Box::new(d)), sent)
    }

    /// Send `blob` with command number `command` to a device whose daemon
    /// answers `answer`, and report the status and what the receiver saw.
    fn send(command: u32, blob: &[u8], answer: Vec<u8>) -> (Status, Vec<Option<Vec<u8>>>) {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        SEEN.with_borrow_mut(|s| *s = Some(Arc::clone(&seen)));
        let (d, _sent) = device_answering(answer);
        // SAFETY: `d` is live, `blob` is a live slice, `record` is sound.
        let status = unsafe {
            tobii_send_custom_command(
                d,
                command,
                blob.as_ptr().cast(),
                blob.len(),
                Some(record),
                ptr::null_mut(),
            )
        };
        // SAFETY: `d` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(d) };
        SEEN.with_borrow_mut(|s| *s = None);
        let saw = seen.lock().expect("seen").clone();
        (status, saw)
    }

    #[test]
    fn a_successful_answer_reaches_the_receiver() {
        let answer = encode_custom_command_answer(tracker_error::NO_ERROR, &[9, 8, 7]);
        let (status, saw) = send(0, &[1, 2, 3], answer);
        assert_eq!(status, TOBII_ERROR_NO_ERROR);
        assert_eq!(saw, vec![Some(vec![9, 8, 7])]);
    }

    #[test]
    fn the_blob_goes_out_as_the_payload_and_only_for_command_zero() {
        let answer = encode_custom_command_answer(tracker_error::NO_ERROR, &[]);
        let (d, sent) = device_answering(answer);
        let blob = [4u8, 5, 6];
        // A non-zero command number is refused with nothing sent, as the
        // DLL's module refuses it before the tracker is touched.
        // SAFETY: `d` is live, `blob` is a live slice, `record` is sound.
        let status = unsafe {
            tobii_send_custom_command(
                d,
                1,
                blob.as_ptr().cast(),
                blob.len(),
                Some(record),
                ptr::null_mut(),
            )
        };
        assert_eq!(status, TOBII_ERROR_NOT_SUPPORTED);
        assert!(sent.lock().expect("sent").is_empty(), "nothing was sent");

        // Zero sends the blob itself: tobiid wraps it in the command's TLV.
        // SAFETY: as above.
        let status = unsafe {
            tobii_send_custom_command(
                d,
                0,
                blob.as_ptr().cast(),
                blob.len(),
                Some(record),
                ptr::null_mut(),
            )
        };
        assert_eq!(status, TOBII_ERROR_NO_ERROR);
        assert_eq!(*sent.lock().expect("sent"), vec![blob.to_vec()]);
        // SAFETY: `d` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(d) };
    }

    #[test]
    fn the_et5_s_probed_refusal_is_an_invalid_parameter() {
        // What the tracker really answered on 2026-10-22, through the DLL's
        // three maps: 0x20000509 -> tracker INVALID_PARAMETER -> platmod 2 ->
        // sesp 8 -> TOBII_ERROR_INVALID_PARAMETER.
        let answer = encode_custom_command_answer(tracker_error::INVALID_PARAMETER, &[]);
        let (status, saw) = send(0, &[0, 0, 0, 0], answer);
        assert_eq!(status, TOBII_ERROR_INVALID_PARAMETER);
        assert!(saw.is_empty(), "a refusal calls no receiver");
    }

    #[test]
    fn every_tracker_result_maps_as_the_dll_s_three_tables_compose() {
        for (result, want) in [
            (tracker_error::NO_ERROR, TOBII_ERROR_NO_ERROR),
            (
                tracker_error::INTERNAL,
                TOBII_ERROR_CONNECTION_FAILED_DRIVER,
            ),
            (
                tracker_error::CONNECTION_FAILED,
                TOBII_ERROR_CONNECTION_FAILED_DRIVER,
            ),
            (
                tracker_error::FIRMWARE_NO_RESPONSE,
                TOBII_ERROR_CONNECTION_FAILED_DRIVER,
            ),
            (tracker_error::NOT_SUPPORTED, TOBII_ERROR_NOT_SUPPORTED),
            (
                tracker_error::INVALID_PARAMETER,
                TOBII_ERROR_INVALID_PARAMETER,
            ),
            (
                tracker_error::ALLOCATION_FAILED,
                TOBII_ERROR_OPERATION_FAILED,
            ),
            (
                tracker_error::OPERATION_FAILED,
                TOBII_ERROR_OPERATION_FAILED,
            ),
            (tracker_error::BUFFER_TOO_SMALL, TOBII_ERROR_INTERNAL),
            (tracker_error::BAD_STATE, TOBII_ERROR_INTERNAL),
            (tracker_error::TOO_MANY_SUBSCRIBERS, TOBII_ERROR_INTERNAL),
        ] {
            assert_eq!(
                custom_command_status(result),
                want,
                "tracker result {result}"
            );
        }
        // Where this composition differs from the custom stream's, which
        // tobiid never reaches but the tables really do differ on.
        assert_ne!(
            custom_command_status(tracker_error::TOO_MANY_SUBSCRIBERS),
            TOBII_ERROR_OPERATION_FAILED,
            "the stream's table says OPERATION_FAILED here, this one INTERNAL"
        );
    }

    #[test]
    fn an_answer_of_no_bytes_reaches_the_receiver_as_a_null_pointer() {
        let answer = encode_custom_command_answer(tracker_error::NO_ERROR, &[]);
        let (status, saw) = send(0, &[1], answer);
        assert_eq!(status, TOBII_ERROR_NO_ERROR);
        assert_eq!(saw, vec![None], "null and zero, as the DLL would");
    }

    #[test]
    fn an_unreadable_answer_is_refused() {
        // A reply too short to hold the tracker result is one tobiid should
        // never send, so it is `malformed`: `TOBII_ERROR_INTERNAL`, as every
        // reply this crate cannot decode. An answer tobiid itself refused --
        // a blob past the bound, or one that is not the single TLV the DLL's
        // schema takes -- arrives as its status instead, `CONNECTION_FAILED`
        // (see `tobiid::custom_command`).
        let (status, saw) = send(0, &[1], vec![0, 0]);
        assert_eq!(status, TOBII_ERROR_INTERNAL);
        assert!(saw.is_empty());

        // And tobiid's own refusal does come through as its status.
        let connect = fake_daemon(|body| match body.first() {
            Some(&tobii_ipc::TAG_SUBSCRIBE) => vec![tobii_ipc::encode_subscribed(true)],
            Some(&tobii_ipc::TAG_REQUEST) => {
                let req = request::decode_request(body).expect("request");
                vec![tobii_ipc::encode_reply(
                    req.id,
                    request::status::CONNECTION_FAILED,
                    &[],
                )]
            }
            _ => vec![],
        });
        let d = Box::into_raw(Box::new(Device::new(connect, 1, 1).expect("device")));
        let blob = [1u8];
        // SAFETY: `d` is live, `blob` is a live slice, `record` is sound.
        let status = unsafe {
            tobii_send_custom_command(d, 0, blob.as_ptr().cast(), 1, Some(record), ptr::null_mut())
        };
        assert_eq!(status, TOBII_ERROR_CONNECTION_FAILED);
        // SAFETY: `d` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(d) };
    }

    #[test]
    fn a_blob_past_the_bound_is_refused_without_a_round_trip() {
        let answer = encode_custom_command_answer(tracker_error::NO_ERROR, &[]);
        let (d, sent) = device_answering(answer);
        let blob = vec![0u8; CUSTOM_COMMAND_MAX + 1];
        // SAFETY: `d` is live, `blob` is a live slice, `record` is sound.
        let status = unsafe {
            tobii_send_custom_command(
                d,
                0,
                blob.as_ptr().cast(),
                blob.len(),
                Some(record),
                ptr::null_mut(),
            )
        };
        assert_eq!(status, TOBII_ERROR_INVALID_PARAMETER);
        assert!(sent.lock().expect("sent").is_empty(), "nothing was sent");

        // The accepting edge: exactly at the bound still goes out.
        let blob = vec![0u8; CUSTOM_COMMAND_MAX];
        // SAFETY: `d` is live, `blob` is a live slice, `record` is sound.
        let status = unsafe {
            tobii_send_custom_command(
                d,
                0,
                blob.as_ptr().cast(),
                blob.len(),
                Some(record),
                ptr::null_mut(),
            )
        };
        assert_eq!(status, TOBII_ERROR_NO_ERROR, "at the bound");
        assert_eq!(
            sent.lock().expect("sent").len(),
            1,
            "and it really was sent"
        );
        // SAFETY: `d` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(d) };
    }

    #[test]
    fn the_arguments_are_checked_in_the_dll_s_order_without_a_round_trip() {
        let (d, frames) = counting_device();
        let blob = [1u8];
        let n: *mut c_void = ptr::null_mut();
        // SAFETY: each call is given a live handle or a null one, and a live
        // slice or a null pointer; `record` is sound.
        unsafe {
            assert_eq!(
                tobii_send_custom_command(
                    ptr::null_mut(),
                    0,
                    blob.as_ptr().cast(),
                    1,
                    Some(record),
                    n
                ),
                TOBII_ERROR_INVALID_PARAMETER,
                "a null device"
            );
            assert_eq!(
                tobii_send_custom_command(d, 0, ptr::null(), 1, Some(record), n),
                TOBII_ERROR_INVALID_PARAMETER,
                "null data"
            );
            assert_eq!(
                tobii_send_custom_command(d, 0, blob.as_ptr().cast(), 0, Some(record), n),
                TOBII_ERROR_INVALID_PARAMETER,
                "a size of zero, which the DLL refuses too"
            );
            assert_eq!(
                tobii_send_custom_command(d, 0, blob.as_ptr().cast(), 1, None, n),
                TOBII_ERROR_INVALID_PARAMETER,
                "a null receiver"
            );
            // The arguments lose to the command gate only after they check
            // out: a bad number with a null blob is still a bad blob.
            assert_eq!(
                tobii_send_custom_command(d, 7, ptr::null(), 1, Some(record), n),
                TOBII_ERROR_INVALID_PARAMETER
            );
            crate::device::call(|| {
                assert_eq!(
                    tobii_send_custom_command(d, 0, blob.as_ptr().cast(), 1, Some(record), n),
                    TOBII_ERROR_CALLBACK_IN_PROGRESS,
                    "the flag, which this crate checks first"
                );
            });
        }
        assert_eq!(frames.load(Ordering::Relaxed), 0, "no daemon round trip");
        destroy_untouched(d, &frames);
    }
}
