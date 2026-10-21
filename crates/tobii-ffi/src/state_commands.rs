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

use std::ffi::{c_char, c_void};

use crate::calibration::request;
use crate::device::{Device, device_ref, in_callback};
use crate::status::{
    Status, TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_INSUFFICIENT_LICENSE,
    TOBII_ERROR_INTERNAL, TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NO_ERROR,
};
use crate::timeouts;
use tobii_ipc::request::{self as ipc, EXTENSIONS_ALL, EXTENSIONS_ENABLED, kind};

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

/// One extension, as the DLL hands it to an extension receiver: 76 bytes,
/// `{u32, u32, char[64], u32}` (its trampoline 0x180001f30 copies a word from
/// +0 and +4, the string to +8, and a word from +0x48; its response parser
/// 0x18017c29d..0x18017c33e keeps records of stride 0x4c and ends the string
/// at +0x47).
///
/// **Provisional**: no extension was ever on the wire — the ET5 has none — so
/// what the fields mean is unknown and their names are ours. libtobii never
/// fills one, since every list it can read is empty.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Extension {
    /// The record's first word, at +0.
    pub word_0: u32,
    /// Its second word, at +4.
    pub word_1: u32,
    /// Its string, NUL-terminated, at +8.
    pub name: [c_char; 64],
    /// Its last word, at +0x48.
    pub word_2: u32,
}

const _: () = assert!(std::mem::size_of::<Extension>() == 0x4c);

/// What an extension enumeration hands each extension to.
pub type ExtensionReceiver = unsafe extern "C" fn(*const Extension, *mut c_void);

/// Ask the daemon for the extensions `selector` names and hand each to
/// `receiver`. The checks come in this crate's order (see `stub::refused`):
/// the callback flag first, then a null device or receiver, all before any
/// request. The DLL checks the nulls first, its callback flag being reachable
/// only through the device; the two differ only for a null argument passed
/// from inside a callback.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
unsafe fn enumerate_extensions(
    device: *mut Device,
    receiver: Option<ExtensionReceiver>,
    selector: u8,
) -> Status {
    if in_callback() {
        return TOBII_ERROR_CALLBACK_IN_PROGRESS;
    }
    if device.is_null() || receiver.is_none() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    // SAFETY: non-null, and the caller guarantees it is live for the call.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    match d.request(kind::EXTENSIONS, &[selector], timeouts::EXTENSIONS) {
        // The daemon answers OK only for an empty list (see `EXTENSIONS`), so
        // the receiver is never called; a count it could not have sent is not
        // a list we can hand on.
        Ok(reply) => match ipc::decode_u32(&reply) {
            Some(0) => TOBII_ERROR_NO_ERROR,
            _ => TOBII_ERROR_INTERNAL,
        },
        Err(status) => status,
    }
}

/// Every extension the tracker has: command 1810, as the DLL's export
/// (0x18014f850) has its module send. The ET5 has none (probe 2026-10-21), so
/// `receiver` is never called and the answer is `TOBII_ERROR_NO_ERROR`.
///
/// The DLL refuses a consumer ET5 before it asks (its module wants licence
/// level 3, 0x180172cea, and a consumer device has 0) with
/// `TOBII_ERROR_INSUFFICIENT_LICENSE`. libtobii checks no licence and answers
/// what lies past that check, as it does for its other reads. A daemon too old
/// to know the request answers `TOBII_ERROR_NOT_SUPPORTED`.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns; `receiver` must be safe to call with `user_data`, though it
/// is never called on an ET5.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_enumerate_extensions(
    device: *mut Device,
    receiver: Option<ExtensionReceiver>,
    _user_data: *mut c_void,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { enumerate_extensions(device, receiver, EXTENSIONS_ALL) }
}

/// The extensions that are switched on: command 1820 (the DLL's export
/// 0x18014f650, its module's licence check at 0x1801730ba). The ET5's answer
/// is the same empty list as for every extension. As
/// `tobii_enumerate_extensions` otherwise.
///
/// # Safety
/// As `tobii_enumerate_extensions`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_enumerate_enabled_extensions(
    device: *mut Device,
    receiver: Option<ExtensionReceiver>,
    _user_data: *mut c_void,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { enumerate_extensions(device, receiver, EXTENSIONS_ENABLED) }
}

/// The answer of `export`, a write the DLL refuses a consumer ET5 on its
/// licence alone, as `stub::refused` answers but with the DLL's code:
/// `TOBII_ERROR_CALLBACK_IN_PROGRESS` inside a callback, then
/// `TOBII_ERROR_INVALID_PARAMETER` for a null device or when an argument the
/// DLL requires is null (`missing`), and otherwise
/// `TOBII_ERROR_INSUFFICIENT_LICENSE`, with nothing written and no daemon
/// round trip. The DLL checks the nulls before its callback flag; as for every
/// entry point here, the two differ only for a null argument passed from
/// inside a callback.
///
/// The DLL gets there without asking the tracker: `device_create` gives a
/// consumer device module licence level 0 (`xor r14d,r14d` at 0x180157a2e is
/// the only write of the level it stores at 0x180158844), and these modules
/// want more before they send anything. The user decided on 2026-10-21 that
/// writes answer as the DLL does, so these keep the DLL's refusal, where reads
/// answer what lies past it. The device is only compared with null.
fn licence_refused(export: &'static str, device: *mut Device, missing: bool) -> Status {
    if in_callback() {
        return TOBII_ERROR_CALLBACK_IN_PROGRESS;
    }
    if device.is_null() || missing {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    tracing::trace!(
        export,
        "insufficient licence, as the DLL for a consumer ET5"
    );
    TOBII_ERROR_INSUFFICIENT_LICENSE
}

/// Set the illumination mode: `TOBII_ERROR_INSUFFICIENT_LICENSE`, as the DLL
/// answers a consumer ET5 (see `licence_refused`).
///
/// The DLL's export (0x180145620) checks a null device, then a null mode
/// (logged, line 0x72), then the callback flag (libtobii checks the flag
/// first, see `licence_refused`), and its module
/// (`platmod_ttp_set_illumination_mode`, 0x180179150) refuses below licence
/// level 1 (`cmp [rcx+0xdac8],0x1`, 0x18017915f) before it would send command
/// 2020. The mode is a NUL-terminated string of at most 64 bytes, which is
/// never read here. Even past that check the ET5 has nothing to set: it does
/// not implement reading the mode either, answering 2010 and 2030 with
/// `0x20000501` (probe 2026-10-21).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_set_illumination_mode(device: *mut Device, mode: *const c_char) -> Status {
    licence_refused("tobii_set_illumination_mode", device, mode.is_null())
}

/// Switch an extension on: `TOBII_ERROR_INSUFFICIENT_LICENSE`, as the DLL
/// answers a consumer ET5 (see `licence_refused`).
///
/// The DLL's export (0x18014f4c0) checks a null device, then the callback
/// flag (libtobii the other way round, see `licence_refused`), and does not
/// range-check `extension`; its module
/// (`platmod_ttp_enable_extension`, 0x180173c50) refuses below licence level 3
/// (0x180173c5a) before it would send command 1800. That refusal is the safe
/// answer twice over: nothing in the DLL can switch an extension off again —
/// there is no disable command, string or builder — and the ET5 has no
/// extensions to switch on (probe 2026-10-21).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_enable_extension(device: *mut Device, extension: u32) -> Status {
    let _ = extension;
    licence_refused("tobii_enable_extension", device, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tobii_device_destroy;
    use crate::status::{
        TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_INSUFFICIENT_LICENSE, TOBII_ERROR_NO_ERROR,
        TOBII_ERROR_NOT_SUPPORTED,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tobii_ipc::request::{self, status};

    /// Requests a fake daemon was sent, as `(kind, payload)`.
    type Requests = Arc<Mutex<Vec<(u8, Vec<u8>)>>>;

    /// A device on a fake daemon answering every request `answer`, and the
    /// requests it was sent.
    fn daemon(answer: u8) -> (*mut Device, Requests) {
        daemon_with(answer, Vec::new())
    }

    /// As [`daemon`], every reply carrying `payload`.
    fn daemon_with(answer: u8, payload: Vec<u8>) -> (*mut Device, Requests) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let connect = crate::device::tests::fake_daemon(move |body| {
            let req = request::decode_request(body).expect("request");
            log.lock()
                .expect("log")
                .push((req.kind, req.payload.to_vec()));
            vec![tobii_ipc::encode_reply(req.id, answer, &payload)]
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
    /// How many times the counting receiver was called.
    static RECEIVED: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn counting(_: *const Extension, _: *mut c_void) {
        RECEIVED.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn the_et5_has_no_extensions_so_the_receiver_is_never_called() {
        let (d, seen) = daemon_with(status::OK, request::encode_u32(0));
        let before = RECEIVED.load(Ordering::Relaxed);
        // SAFETY: `d` is live and destroyed once; the receiver is sound.
        unsafe {
            let null = std::ptr::null_mut();
            assert_eq!(
                tobii_enumerate_extensions(d, Some(counting), null),
                TOBII_ERROR_NO_ERROR
            );
            assert_eq!(
                tobii_enumerate_enabled_extensions(d, Some(counting), null),
                TOBII_ERROR_NO_ERROR
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }
        assert_eq!(RECEIVED.load(Ordering::Relaxed), before, "never called");
        assert_eq!(
            *seen.lock().expect("log"),
            [
                (kind::EXTENSIONS, vec![EXTENSIONS_ALL]),
                (kind::EXTENSIONS, vec![EXTENSIONS_ENABLED])
            ]
        );
    }

    #[test]
    fn a_null_device_or_receiver_is_refused_before_any_request() {
        let (d, seen) = daemon_with(status::OK, request::encode_u32(0));
        // SAFETY: `d` is live and destroyed once.
        unsafe {
            let user = std::ptr::null_mut();
            assert_eq!(
                tobii_enumerate_extensions(std::ptr::null_mut(), Some(counting), user),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_enumerate_extensions(d, None, user),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }
        assert!(seen.lock().expect("log").is_empty());
    }

    #[test]
    fn the_callback_flag_comes_before_a_null_receiver_as_everywhere_here() {
        // This crate's order (see `stub::refused`); the DLL would answer the
        // null receiver INVALID_PARAMETER even inside a callback.
        let (d, _) = daemon_with(status::OK, request::encode_u32(0));
        let mut got = (0, 0);
        // SAFETY: `d` is live and destroyed once.
        crate::device::call(|| unsafe {
            let null = std::ptr::null_mut();
            got = (
                tobii_enumerate_extensions(d, None, null),
                tobii_enumerate_extensions(d, Some(counting), null),
            );
        });
        // SAFETY: as above, outside the callback.
        unsafe { assert_eq!(tobii_device_destroy(d), 0) };
        assert_eq!(
            got,
            (
                TOBII_ERROR_CALLBACK_IN_PROGRESS,
                TOBII_ERROR_CALLBACK_IN_PROGRESS
            )
        );
    }

    #[test]
    fn extensions_on_a_daemon_that_predates_them_are_not_supported() {
        let (d, _) = daemon(status::NOT_SUPPORTED);
        // SAFETY: `d` is live and destroyed once.
        unsafe {
            assert_eq!(
                tobii_enumerate_extensions(d, Some(counting), std::ptr::null_mut()),
                TOBII_ERROR_NOT_SUPPORTED
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    #[test]
    fn a_count_the_daemon_could_not_have_sent_is_internal() {
        let (d, _) = daemon_with(status::OK, request::encode_u32(1));
        // SAFETY: `d` is live and destroyed once.
        unsafe {
            assert_eq!(
                tobii_enumerate_extensions(d, Some(counting), std::ptr::null_mut()),
                TOBII_ERROR_INTERNAL
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }
    #[test]
    fn the_writes_are_refused_on_the_licence_without_asking_the_daemon() {
        let (d, seen) = daemon(status::OK);
        let mode = c"Default".as_ptr();
        assert_eq!(
            tobii_set_illumination_mode(d, mode),
            TOBII_ERROR_INSUFFICIENT_LICENSE
        );
        assert_eq!(
            tobii_enable_extension(d, 9000),
            TOBII_ERROR_INSUFFICIENT_LICENSE
        );
        // No range check on the extension, as in the DLL.
        assert_eq!(
            tobii_enable_extension(d, u32::MAX),
            TOBII_ERROR_INSUFFICIENT_LICENSE
        );
        // SAFETY: `d` is live and destroyed once.
        unsafe { assert_eq!(tobii_device_destroy(d), 0) };
        assert!(seen.lock().expect("log").is_empty(), "no daemon round trip");
    }

    #[test]
    fn the_writes_check_the_callback_flag_before_their_nulls_as_everywhere_here() {
        let (d, _) = daemon(status::OK);
        let mode = c"Default".as_ptr();
        assert_eq!(
            tobii_set_illumination_mode(std::ptr::null_mut(), mode),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_set_illumination_mode(d, std::ptr::null()),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_enable_extension(std::ptr::null_mut(), 1),
            TOBII_ERROR_INVALID_PARAMETER
        );
        let mut inside = (0, 0, 0);
        crate::device::call(|| {
            inside = (
                // The DLL would answer this one INVALID_PARAMETER.
                tobii_set_illumination_mode(d, std::ptr::null()),
                tobii_set_illumination_mode(d, mode),
                tobii_enable_extension(std::ptr::null_mut(), 1),
            );
        });
        // SAFETY: `d` is live and destroyed once, outside the callback.
        unsafe { assert_eq!(tobii_device_destroy(d), 0) };
        assert_eq!(
            inside,
            (
                TOBII_ERROR_CALLBACK_IN_PROGRESS,
                TOBII_ERROR_CALLBACK_IN_PROGRESS,
                TOBII_ERROR_CALLBACK_IN_PROGRESS
            )
        );
    }
}
