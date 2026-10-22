//! The diagnostics, log and clean-IR entry points of `tobii_internal.h`, and
//! what they answer for an ET5.
//!
//! All eight are decided without the tracker, each for its own reason, and
//! the DLL reaches two different answers by two different routes (addresses
//! and the whole derivation:
//! `fixtures/analysis/unknown-protocols-2026-10-22/REPORT-A.md` §1-2.2, and
//! `diag-image-cleanir/REPORT.md`, `diag-log-cmds/REPORT.md` beside it):
//!
//! - **`TOBII_ERROR_NOT_SUPPORTED`** for the diagnostics image pair, whose
//!   stream the ET5's catalogue does not list, and for the two retrieves,
//!   whose PRP command the tracker module never publishes. No licence is
//!   checked anywhere on either route.
//! - **`TOBII_ERROR_INSUFFICIENT_LICENSE`** for clean IR and for the two
//!   commands behind `dump_images` and `get_data`, whose module functions
//!   test the licence level **before** anything else, so an ET5 never reaches
//!   their catalogue check or their command.
//!
//! The split matters: a reader expecting "diagnostics are not supported"
//! would get clean IR wrong, because its licence gate (0x180177364) fires
//! before the check for stream 0x502 that the ET5's catalogue lacks.
//!
//! Nothing here asks the daemon or reads an argument beyond comparing it with
//! null, so every entry point is a safe `extern "C"` function.

use std::ffi::c_void;

use crate::device::Device;
use crate::status::Status;
use crate::stub::{licence_refused, refused};

/// Subscribe to the tracker's diagnostics image:
/// `TOBII_ERROR_NOT_SUPPORTED` for an ET5.
///
/// The DLL's export (0x18014de20) passes Stream Engine stream 7 to the generic
/// `tobii_stream_subscribe` (0x18015cbf0), which answers
/// `TOBII_ERROR_INVALID_PARAMETER` for a null device (unlogged, 0x18015cc1b)
/// or callback (`core\internal.cpp` line 411),
/// `TOBII_ERROR_CALLBACK_IN_PROGRESS` inside a callback (line 412), and then
/// `TOBII_ERROR_NOT_SUPPORTED` (0x18015ccbd, line 413) when its support check
/// (0x1801591b0) fails. On the DLL's own tracker module that check asks the
/// catalogue the module read at connect for stream type 8, which is device
/// stream 0x506 (the type-to-id switch 0x1801801a0, `mov eax,0x506` at
/// 0x1801801ed). An ET5's catalogue lists no 0x506, so the answer is settled
/// before any request; a device whose catalogue could not be read at all
/// answers the same, the module's lookup returning false for an empty
/// catalogue (0x18017029e).
///
/// No licence is checked on this route: neither the export, the generic
/// subscribe, the support check, the subscribe worker (0x1801536d0) nor the
/// module's own subscribe (0x180161150) reads the module licence level.
/// Stream 7 maps to PRP stream 22, `DIAGNOSTICS_IMAGE` (map 0x180153a50,
/// name switch 0x1800232a3), and the application would be handed a 0xa0-byte
/// record (`tobii_internal.h`); the ET5 never sends one, so libtobii carries
/// no decoder for it.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_diagnostics_image_subscribe(
    device: *mut Device,
    callback: *const c_void,
    user_data: *mut c_void,
) -> Status {
    let _ = user_data;
    refused(
        "tobii_diagnostics_image_subscribe",
        device,
        callback.is_null(),
    )
}

/// Undo `tobii_diagnostics_image_subscribe`:
/// `TOBII_ERROR_NOT_SUPPORTED` for an ET5, never
/// `TOBII_ERROR_NOT_SUBSCRIBED`.
///
/// The DLL's export (0x18014dd90) passes stream 7 to
/// `tobii_stream_unsubscribe` (0x18015cee0), which checks a null device
/// (unlogged) and the callback flag (line 425) and then makes the **same**
/// support check as the subscribe (0x18015cf34, line 426), before it ever
/// looks for a subscription: `TOBII_ERROR_NOT_SUBSCRIBED` comes only from the
/// unsubscribe worker further down (0x180153595, `device.cpp` line 2434),
/// which an ET5 never reaches. So an unsubscribe without a subscribe answers
/// not-supported too, and the order is the DLL's, not this crate's choice.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_diagnostics_image_unsubscribe(device: *mut Device) -> Status {
    refused("tobii_diagnostics_image_unsubscribe", device, false)
}

/// Retrieve the diagnostic images the tracker holds:
/// `TOBII_ERROR_NOT_SUPPORTED` for an ET5.
///
/// The DLL's export (0x18014b2b0) checks a null device
/// (`TOBII_ERROR_INVALID_PARAMETER`, unlogged, 0x18014b2cb), then a null
/// receiver (`tobii_internal.cpp` line 722), then the callback flag (line
/// 723), and then scans the device's PRP command list (`[dev+0x88c0]`, count
/// `[dev+0x8930]`, from the connect message) for command **0x15**
/// (`cmp [rax],0x15` at 0x18014b380), answering
/// `TOBII_ERROR_NOT_SUPPORTED` when it is absent (0x18014b3f6, line 725).
///
/// The legacy TTP module never publishes it. `platmod_start` (0x18016ac30)
/// lists its own commands 0xa and 0xb always, 0..6 for a generation other
/// than `VR*` and `EyeXlite`, then 9, and 0xc/0xd only when its
/// persistent-file enumeration (command 3060) succeeded at connect; the
/// server's module-to-PRP table (0x18001f874) maps none of those to 0x15. So
/// an ET5 is answered before any request, and the module's own handler
/// (`platmod_command_diagnostic_images_retrieve` 0x180167600) would refuse it
/// again anyway, logging `PLATMOD_ERROR_NOT_SUPPORTED` and returning 3.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_diagnostic_images_retrieve(
    device: *mut Device,
    receiver: *const c_void,
    user_data: *mut c_void,
) -> Status {
    let _ = user_data;
    refused(
        "tobii_diagnostic_images_retrieve",
        device,
        receiver.is_null(),
    )
}

/// Retrieve the tracker's log files: `TOBII_ERROR_NOT_SUPPORTED` for an ET5.
///
/// As `tobii_diagnostic_images_retrieve`, for PRP command **0x19**: the
/// export (0x18014ae90) checks a null device (unlogged, 0x18014aeab), a null
/// receiver (line 779), the callback flag (line 780), and then the command
/// list (`cmp [rax],0x19` at 0x18014af60), answering not-supported at
/// 0x18014afd6 (line 782). The module publishes no command that maps to
/// 0x19, and `platmod_command_log_files_retrieve` (0x180167570) returns 3
/// regardless.
///
/// This export has nothing to do with device stream 0x1772, the catalogue's
/// `log` stream: no export of the DLL reaches that stream at all. The one
/// reading that suggested otherwise mistook the log line numbers 6002 and
/// 6004 of `platmod_legacy_ttp.cpp` (the immediates 0x1772 and 0x1774 at
/// 0x1801424e3, 0x180142559, 0x18017a592 and 0x18017a602, all inside
/// `platmod_ttp_get_state_warning`) for stream ids.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_logs_retrieve(
    device: *mut Device,
    receiver: *const c_void,
    user_data: *mut c_void,
) -> Status {
    let _ = user_data;
    refused("tobii_logs_retrieve", device, receiver.is_null())
}

/// Have the tracker dump its diagnostic images:
/// `TOBII_ERROR_INSUFFICIENT_LICENSE` for an ET5, and no command sent.
///
/// The DLL's export (0x18014e710) checks a null device
/// (`TOBII_ERROR_INVALID_PARAMETER`, unlogged, 0x18014e72a), then the
/// callback flag (`tobii_internal.cpp` line 216), takes the API mutex, and
/// answers `TOBII_ERROR_NOT_SUPPORTED` with no tracker module of its own
/// (line 223) — behind Tobii's service. On the module,
/// `platmod_ttp_diagnostics_dump_images` (0x1801744b0) tests the licence
/// level as its **first** instruction (`cmp [rcx+0xdac8],3` at 0x1801744ba)
/// and returns `TOBII_ERROR_INSUFFICIENT_LICENSE` (0x18017452c,
/// `platmod_legacy_ttp.cpp` line 7072) below the internal level. A consumer
/// ET5 is level 0, so **command 1500 never goes out** and its two `u32`
/// arguments are never read — which is as well, since nothing in the DLL
/// names them, bounds them or gives them a default, and no capture holds the
/// command.
///
/// Note for anyone tempted to implement it past the gate: 1500 and 1510 are
/// the only two of the DLL's 110 `send_and_retrieve_response` call sites that
/// pass a timeout of **zero**, so they wait for the tracker without a
/// deadline.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_diagnostics_dump_images(device: *mut Device, a: u32, b: u32) -> Status {
    let _ = (a, b);
    licence_refused("tobii_diagnostics_dump_images", device, false)
}

/// Read a kind of diagnostics data from the tracker:
/// `TOBII_ERROR_INSUFFICIENT_LICENSE` for an ET5, and no command sent.
///
/// The DLL's export (0x18014e510) checks a null device (unlogged,
/// 0x18014e534), then a null receiver (`tobii_internal.cpp` line 231), then
/// the callback flag (line 232), takes the API mutex, and answers
/// `TOBII_ERROR_NOT_SUPPORTED` with no tracker module of its own (line 239).
/// On the module, `platmod_ttp_diagnostics_get_data` (0x180176b60) tests the
/// licence level first (`cmp [rcx+0xdac8],3` at 0x180176b6a) and returns
/// `TOBII_ERROR_INSUFFICIENT_LICENSE` (0x180176bdc, line 7101). So **command
/// 1510 never goes out**, and the `kind` the caller asks for is never read;
/// which values it would accept is not established anywhere.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_diagnostics_get_data(
    device: *mut Device,
    kind: u32,
    receiver: *const c_void,
    user_data: *mut c_void,
) -> Status {
    let _ = (kind, user_data);
    licence_refused("tobii_diagnostics_get_data", device, receiver.is_null())
}

/// Subscribe to the clean infrared image:
/// `TOBII_ERROR_INSUFFICIENT_LICENSE` for an ET5.
///
/// Not `TOBII_ERROR_NOT_SUPPORTED`, which is what the stream's absence from
/// the catalogue would suggest: the DLL decides on the licence first. The
/// export (0x18014ee30) checks a null device (unlogged, 0x18014ee43), a null
/// callback (`tobii_internal.cpp` line 147), the callback flag (line 148) and
/// then, with no tracker module of its own, `TOBII_ERROR_NOT_SUPPORTED`
/// (line 155) — behind Tobii's service. On the module,
/// `platmod_ttp_clean_ir_subscribe` (0x180177350) tests the licence level
/// first (`cmp [rcx+0xdac8],3` at 0x180177364, refusal at line 6993), and
/// only past it would it refuse a taken slot
/// (`TOBII_ERROR_ALREADY_SUBSCRIBED`, line 6995), then a catalogue without
/// stream type 3 — device stream 0x502 (0x180177460, line 6997) — and only
/// then send a 1220. An ET5 has neither the licence nor 0x502, and the
/// licence answer is the one it gets.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_clean_ir_subscribe(
    device: *mut Device,
    callback: *const c_void,
    user_data: *mut c_void,
) -> Status {
    let _ = user_data;
    licence_refused("tobii_clean_ir_subscribe", device, callback.is_null())
}

/// Undo `tobii_clean_ir_subscribe`: `TOBII_ERROR_INSUFFICIENT_LICENSE` for an
/// ET5, never `TOBII_ERROR_NOT_SUBSCRIBED`.
///
/// The export (0x18014ec60) checks a null device (unlogged), the callback
/// flag, and `TOBII_ERROR_NOT_SUPPORTED` with no tracker module of its own
/// (`tobii_internal.cpp` line 169). The module's unsubscribe
/// (0x1801763b0) tests the licence level first (`cmp [rcx+0xdac8],3` at
/// 0x1801763ce, refusal at line 7034), so it never reaches its
/// `TOBII_ERROR_NOT_SUBSCRIBED`; unlike the subscribe it makes no catalogue
/// check at all.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_clean_ir_unsubscribe(device: *mut Device) -> Status {
    licence_refused("tobii_clean_ir_unsubscribe", device, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::{counting_device, destroy_untouched};
    use crate::status::{
        TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_INSUFFICIENT_LICENSE,
        TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NOT_SUPPORTED,
    };
    use std::ptr;

    /// A callback or receiver the refused calls are given and never call.
    fn never_called() -> *const c_void {
        ptr::NonNull::<c_void>::dangling().as_ptr().cast_const()
    }

    #[test]
    fn a_null_device_is_an_invalid_parameter_for_every_entry_point() {
        assert_eq!(
            tobii_diagnostics_image_subscribe(ptr::null_mut(), never_called(), ptr::null_mut()),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_diagnostics_image_unsubscribe(ptr::null_mut()),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_diagnostic_images_retrieve(ptr::null_mut(), never_called(), ptr::null_mut()),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_logs_retrieve(ptr::null_mut(), never_called(), ptr::null_mut()),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_diagnostics_dump_images(ptr::null_mut(), 0, 0),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_diagnostics_get_data(ptr::null_mut(), 0, never_called(), ptr::null_mut()),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_clean_ir_subscribe(ptr::null_mut(), never_called(), ptr::null_mut()),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_clean_ir_unsubscribe(ptr::null_mut()),
            TOBII_ERROR_INVALID_PARAMETER
        );
    }

    #[test]
    fn the_required_argument_of_each_entry_point_is_checked() {
        // The DLL requires a callback or receiver of four of the eight, and
        // of no other: dump_images takes two words it never reads, and the
        // two unsubscribes take nothing.
        let (d, asked) = counting_device();
        for answer in [
            tobii_diagnostics_image_subscribe(d, ptr::null(), ptr::null_mut()),
            tobii_diagnostic_images_retrieve(d, ptr::null(), ptr::null_mut()),
            tobii_logs_retrieve(d, ptr::null(), ptr::null_mut()),
            tobii_diagnostics_get_data(d, 0, ptr::null(), ptr::null_mut()),
            tobii_clean_ir_subscribe(d, ptr::null(), ptr::null_mut()),
        ] {
            assert_eq!(answer, TOBII_ERROR_INVALID_PARAMETER);
        }
        assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 0);
        destroy_untouched(d, &asked);
    }

    #[test]
    fn the_two_routes_answer_as_the_dll_does_for_an_et5() {
        let (d, asked) = counting_device();
        // No catalogue, no licence: not-supported.
        for answer in [
            tobii_diagnostics_image_subscribe(d, never_called(), ptr::null_mut()),
            tobii_diagnostics_image_unsubscribe(d),
            tobii_diagnostic_images_retrieve(d, never_called(), ptr::null_mut()),
            tobii_logs_retrieve(d, never_called(), ptr::null_mut()),
        ] {
            assert_eq!(answer, TOBII_ERROR_NOT_SUPPORTED);
        }
        // The licence gate comes first for these four.
        for answer in [
            tobii_diagnostics_dump_images(d, 0, 0),
            tobii_diagnostics_get_data(d, 0, never_called(), ptr::null_mut()),
            tobii_clean_ir_subscribe(d, never_called(), ptr::null_mut()),
            tobii_clean_ir_unsubscribe(d),
        ] {
            assert_eq!(answer, TOBII_ERROR_INSUFFICIENT_LICENSE);
        }
        // None of the eight asks the daemon anything.
        assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 0);
        destroy_untouched(d, &asked);
    }

    #[test]
    fn a_call_from_inside_a_callback_is_refused_before_anything_else() {
        let (d, asked) = counting_device();
        crate::device::call(|| {
            // Even a null argument loses to the flag here, which is this
            // crate's order; the DLL checks its nulls first.
            assert_eq!(
                tobii_clean_ir_subscribe(d, ptr::null(), ptr::null_mut()),
                TOBII_ERROR_CALLBACK_IN_PROGRESS
            );
            assert_eq!(
                tobii_diagnostics_image_unsubscribe(d),
                TOBII_ERROR_CALLBACK_IN_PROGRESS
            );
            assert_eq!(
                tobii_diagnostics_dump_images(d, 7, 9),
                TOBII_ERROR_CALLBACK_IN_PROGRESS
            );
        });
        assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 0);
        destroy_untouched(d, &asked);
    }
}
