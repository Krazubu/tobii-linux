//! The display-id and display-info entry points of `tobii_internal.h`, and
//! what they answer for an ET5.
//!
//! All five are decided without the tracker, and again by two routes with two
//! answers (the derivation and every address:
//! `fixtures/analysis/unknown-protocols-2026-10-22/REPORT-A.md` §1 and §2.3,
//! with `display/REPORT.md` beside it):
//!
//! - the four display-id exports are PRP property **0x12**, which the legacy
//!   TTP module publishes in none of its readable, writable or subscribable
//!   lists, so the DLL's own property layer answers
//!   `TOBII_ERROR_NOT_SUPPORTED` before any request;
//! - `tobii_get_display_info` answers **`TOBII_ERROR_NOT_AVAILABLE`**,
//!   because its module function wants Tobii's Stream Engine Services, which
//!   do not exist on Linux. It is the same premise `tobii_send_statistics`
//!   already rests on, and the same answer.
//!
//! **The display id is a string, not a `uint32_t`.** The DLL's setter copies
//! 0x100 bytes of it with `strncpy` (0x1801466cb) before it checks anything,
//! and the getter's output is a 256-byte buffer. The old prototype in
//! `tobii_internal.h` would have had a real caller dereference a small
//! integer as a `char*`.
//!
//! Where this crate deliberately differs from the DLL, besides the usual
//! callback-flag order: the DLL enters its property layer **before** its own
//! null-device test in the getter, the subscribe and the unsubscribe
//! (0x1801467c1 before 0x1801467c6, and so on) — only the setter tests first,
//! because `tobii_property_set` is the one property entry point with no
//! null-device check of its own. The returned code is the same either way
//! (`lea eax,[rdi+8]` overwrites the layer's answer), so only the DLL's log
//! output differs, including a double log when a subscribe is given a live
//! device and a null callback. And this crate reads no string before
//! refusing, where the DLL copies one: nothing is read here at all.

use std::ffi::{c_char, c_void};

use crate::device::Device;
use crate::status::{Status, TOBII_ERROR_NOT_AVAILABLE};
use crate::stub::refused;

/// Subscribe to changes of the tracker's display id:
/// `TOBII_ERROR_NOT_SUPPORTED` for an ET5.
///
/// The DLL's export (0x180146600) hands PRP property 0x12 to
/// `tobii_property_subscribe` (0x18015c860, the id at 0x18014664d), which
/// answers `TOBII_ERROR_INVALID_PARAMETER` for a null device or callback
/// (`core\internal.cpp` line 467), `TOBII_ERROR_CALLBACK_IN_PROGRESS` inside a
/// callback (line 468), and then `TOBII_ERROR_NOT_SUPPORTED` (0x18015c9a3,
/// `core\internal.cpp` line 470) when the device's subscribable list
/// (`[dev+0x87ac]`, count `[dev+0x87fc]`, checked by 0x1801538f0) lacks the
/// property. `platmod_start` never puts 0x12 in that list, so an ET5 is
/// refused before anything is sent. The callback would have been handed the
/// id as a `char const*` with its user data.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_display_id_subscribe(
    device: *mut Device,
    callback: *const c_void,
    user_data: *mut c_void,
) -> Status {
    let _ = user_data;
    refused("tobii_display_id_subscribe", device, callback.is_null())
}

/// Undo `tobii_display_id_subscribe`: `TOBII_ERROR_NOT_SUPPORTED` for an ET5,
/// never `TOBII_ERROR_NOT_SUBSCRIBED`.
///
/// The DLL's export (0x180146570) goes through `tobii_property_unsubscribe`
/// (0x18015c410), which checks a null device, the callback flag
/// (`core\internal.cpp` line 516) and then the same property list
/// (0x18015c4dd, line 518) **before** it looks for a subscription — its
/// `TOBII_ERROR_NOT_SUBSCRIBED` (line 524) is unreachable for a property the
/// device does not publish.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_display_id_unsubscribe(device: *mut Device) -> Status {
    refused("tobii_display_id_unsubscribe", device, false)
}

/// Read the tracker's display id: `TOBII_ERROR_NOT_SUPPORTED` for an ET5,
/// with `display_id` left untouched.
///
/// The DLL's export (0x180146770) hands property 0x12 to
/// `tobii_property_get` (0x18015d300, the id at 0x1801467b9), which answers
/// `TOBII_ERROR_INVALID_PARAMETER` for a null device (unlogged) or output
/// (`core\internal.cpp` line 558), `TOBII_ERROR_CALLBACK_IN_PROGRESS` inside a
/// callback (line 559), and then `TOBII_ERROR_NOT_SUPPORTED` (0x18015d450,
/// line 560) when the device's readable list (`[dev+0x8704]`, count
/// `[dev+0x8754]`) lacks it — which an ET5's always does.
///
/// `display_id` is a 256-byte `char` buffer, not a `uint32_t*`; nothing is
/// written to it on this path, and nothing is read from it here.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_get_display_id(device: *mut Device, display_id: *mut c_char) -> Status {
    refused("tobii_get_display_id", device, display_id.is_null())
}

/// Set the tracker's display id: `TOBII_ERROR_NOT_SUPPORTED` for an ET5, and
/// nothing written anywhere.
///
/// The DLL's export (0x1801466b0) copies 0x100 bytes of the **string** it is
/// given into its own frame (`strncpy` at 0x1801466cb) before it checks
/// anything, then tests a null device, then hands property 0x12 to
/// `tobii_property_set` (0x18015c150, the id at 0x18014670c), which checks
/// the callback flag and then answers `TOBII_ERROR_NOT_SUPPORTED`
/// (0x18015c29a, `core\internal.cpp` line 574) when the device's writable
/// list (`[dev+0x8758]`, count `[dev+0x87a8]`) lacks the property. An ET5's
/// does, so no command reaches the tracker and nothing is stored.
///
/// This crate reads no string at all: the DLL's copy happens before its own
/// null-device test, so a null `display_id` with a live device is a fault in
/// the DLL that there is no reason to reproduce (as for
/// `tobii_send_statistics`, nothing here reads past a pointer it only
/// compares with null).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_set_display_id(device: *mut Device, display_id: *const c_char) -> Status {
    refused("tobii_set_display_id", device, display_id.is_null())
}

/// What the host knows about the display the tracker sits at:
/// `TOBII_ERROR_NOT_AVAILABLE` on Linux, with `display_info` left untouched.
///
/// The DLL's export (0x18014dac0) checks a null device
/// (`TOBII_ERROR_INVALID_PARAMETER`, unlogged), then a null output
/// (`tobii_internal.cpp` line 331), then the callback flag (line 332), takes
/// the API mutex, and answers `TOBII_ERROR_NOT_SUPPORTED` with no tracker
/// module of its own (line 339). Its module function
/// `platmod_ttp_get_display_info` (0x180175660) then answers
/// `TOBII_ERROR_NOT_AVAILABLE` straight away, unlogged (0x1801756fd,
/// 0x180175706), unless the module reached Tobii's Stream Engine Services at
/// connect (`[module+0xa850]`, set at 0x18016b33d). Past that it is not a
/// tracker command at all but an IPC request to the services (kind 9, reply
/// type 10) which fills a 72-byte record: a 64-byte display name and two
/// `int32`s whose meaning is not established.
///
/// There are no such services on Linux, so this is the answer, exactly as
/// `tobii_send_statistics` answers `TOBII_ERROR_NOT_AVAILABLE` for the same
/// reason. Unlike the DLL this crate makes no licence check and no daemon
/// round trip, and the record is never written.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_get_display_info(device: *mut Device, display_info: *mut c_void) -> Status {
    // The arguments are checked in this crate's order (the callback flag
    // first), and then the answer is the module's, not the export's: the
    // services the export would hand the call to are not there.
    match refused("tobii_get_display_info", device, display_info.is_null()) {
        crate::status::TOBII_ERROR_NOT_SUPPORTED => TOBII_ERROR_NOT_AVAILABLE,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::{counting_device, destroy_untouched};
    use crate::status::{
        TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NOT_SUPPORTED,
    };
    use std::ptr;
    use std::sync::atomic::Ordering;

    /// A callback the refused calls are given and never call.
    fn never_called() -> *const c_void {
        ptr::NonNull::<c_void>::dangling().as_ptr().cast_const()
    }

    /// A 256-byte display id, the buffer size the DLL's getter writes and its
    /// setter copies. No call here may touch it.
    const DISPLAY_ID: [c_char; 256] = [0x5a; 256];

    #[test]
    fn the_four_display_id_exports_are_not_supported_without_asking_the_daemon() {
        let (d, frames) = counting_device();
        let mut id = DISPLAY_ID;
        let out = id.as_mut_ptr();
        for answer in [
            tobii_display_id_subscribe(d, never_called(), ptr::null_mut()),
            tobii_display_id_unsubscribe(d),
            tobii_get_display_id(d, out),
            tobii_set_display_id(d, out.cast_const()),
        ] {
            assert_eq!(answer, TOBII_ERROR_NOT_SUPPORTED);
        }
        assert_eq!(frames.load(Ordering::Relaxed), 0, "no daemon round trip");
        assert_eq!(id, DISPLAY_ID, "nothing is written to the buffer");
        destroy_untouched(d, &frames);
    }

    #[test]
    fn the_display_info_is_not_available_as_linux_has_no_such_services() {
        let (d, frames) = counting_device();
        let mut info = [0u8; 72];
        assert_eq!(
            tobii_get_display_info(d, info.as_mut_ptr().cast()),
            TOBII_ERROR_NOT_AVAILABLE,
            "the module's answer, not the export's NOT_SUPPORTED"
        );
        assert_eq!(info, [0u8; 72], "the record is never written");
        assert_eq!(frames.load(Ordering::Relaxed), 0);
        destroy_untouched(d, &frames);
    }

    #[test]
    fn a_null_device_or_required_argument_is_an_invalid_parameter() {
        let n: *mut c_void = ptr::null_mut();
        let mut id = DISPLAY_ID;
        let out = id.as_mut_ptr();
        let (d, frames) = counting_device();
        for answer in [
            // A null device, whatever else is passed.
            tobii_display_id_subscribe(n.cast(), never_called(), n),
            tobii_display_id_unsubscribe(n.cast()),
            tobii_get_display_id(n.cast(), out),
            tobii_set_display_id(n.cast(), out.cast_const()),
            tobii_get_display_info(n.cast(), n),
            // A null argument the DLL requires, with a live device.
            tobii_display_id_subscribe(d, ptr::null(), n),
            tobii_get_display_id(d, ptr::null_mut()),
            tobii_set_display_id(d, ptr::null()),
            tobii_get_display_info(d, ptr::null_mut()),
        ] {
            assert_eq!(answer, TOBII_ERROR_INVALID_PARAMETER);
        }
        assert_eq!(frames.load(Ordering::Relaxed), 0);
        destroy_untouched(d, &frames);
    }

    #[test]
    fn a_call_from_inside_a_callback_is_refused_first() {
        let (d, frames) = counting_device();
        let mut info = [0u8; 72];
        crate::device::call(|| {
            assert_eq!(
                tobii_display_id_subscribe(d, ptr::null(), ptr::null_mut()),
                TOBII_ERROR_CALLBACK_IN_PROGRESS,
                "the flag beats even a null argument, as everywhere here"
            );
            assert_eq!(
                tobii_get_display_id(d, ptr::null_mut()),
                TOBII_ERROR_CALLBACK_IN_PROGRESS
            );
            assert_eq!(
                tobii_get_display_info(d, info.as_mut_ptr().cast()),
                TOBII_ERROR_CALLBACK_IN_PROGRESS,
                "not NOT_AVAILABLE: the flag comes first"
            );
        });
        assert_eq!(frames.load(Ordering::Relaxed), 0);
        destroy_untouched(d, &frames);
    }
}
