//! `tobii_wearable.h`: head-mounted devices only. Nothing here applies to the
//! screen-based ET5: the lens-configuration writability query answers "not
//! writable", and every other entry point checks its arguments and then
//! answers `TOBII_ERROR_NOT_SUPPORTED`, as the DLL does for an ET5 it drives
//! itself.
//!
//! The DLL refuses the three wearable streams in its stream checks, which on
//! its own tracker module refuse the advanced and foveated streams outright
//! and offer the consumer stream only to a tracker whose stream catalogue
//! (command 1200) holds stream 0x505, and the lens configuration in its
//! property lists, which that module fills from what the tracker reported at
//! connect, offering it only to a tracker whose generation starts with "VR"
//! (0x18016b831..0x18016b86d). The ET5's catalogue has no 0x505 and its
//! generation is IS5. No licence is checked on the way. So nothing is sent to
//! the daemon and nothing is written.

use std::ffi::c_void;

use crate::device::{Device, device_ref};
use crate::status::{Status, TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NO_ERROR};
use crate::stub::refused;
use crate::types::TOBII_LENS_CONFIGURATION_NOT_WRITABLE;

/// Whether the lens configuration can be written: never on the ET5.
///
/// The DLL answers from the device's list of writable properties (lens
/// configuration is property 0xa) and has no `TOBII_ERROR_NOT_SUPPORTED`
/// path. The ET5 has no lens configuration, so the answer is
/// `TOBII_LENS_CONFIGURATION_NOT_WRITABLE`.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `writable` must be null or
/// valid for writing one `tobii_lens_configuration_writable_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_lens_configuration_writable(
    device: *mut Device,
    writable: *mut u32,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    if let Err(status) = unsafe { device_ref(device) } {
        return status;
    }
    if writable.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    // SAFETY: non-null, and the caller guarantees it is writable.
    unsafe { writable.write(TOBII_LENS_CONFIGURATION_NOT_WRITABLE) };
    TOBII_ERROR_NO_ERROR
}

/// The consumer wearable stream (`tobii_stream_t` 9):
/// `TOBII_ERROR_NOT_SUPPORTED` once the device and callback check out (see
/// [`refused`]).
///
/// The DLL's export (0x180152ed0) checks the device, then hands stream 9 to
/// its `tobii_compound_stream_subscribe` (0x18015b750) for a device that
/// lists no wearable compound stream (`[dev+0x8934]`), as on its own tracker
/// module for a catalogue without stream 0x505 (0x18016ba2a). That checks the
/// callback, then the callback flag, then the stream (0x1801591b0, case 9:
/// the same 0x505 test) and answers `TOBII_ERROR_NOT_SUPPORTED` (0x18015b88e).
/// The callback is only compared with null, so the function is safe.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_wearable_consumer_data_subscribe(
    device: *mut Device,
    callback: *const c_void,
    _user_data: *mut c_void,
) -> Status {
    let export = "tobii_wearable_consumer_data_subscribe";
    refused(export, device, callback.is_null())
}

/// The unsubscribe of `tobii_wearable_consumer_data_subscribe`:
/// `TOBII_ERROR_NOT_SUPPORTED` once the device checks out, never
/// `TOBII_ERROR_NOT_SUBSCRIBED`, as in the DLL (0x180152be0), whose
/// `tobii_compound_stream_unsubscribe` (0x18015b340) checks the stream
/// (0x18015b409) before it looks for a subscription.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_wearable_consumer_data_unsubscribe(device: *mut Device) -> Status {
    refused("tobii_wearable_consumer_data_unsubscribe", device, false)
}

/// The advanced wearable stream (`tobii_stream_t` 10):
/// `TOBII_ERROR_NOT_SUPPORTED` once the device and callback check out, as
/// the DLL (0x180152820) answers through `tobii_compound_stream_subscribe`
/// for stream 10, which its stream check refuses outright on its own tracker
/// module (0x1801591ce). Its licence check (feature group professional or
/// above, 0x180152a0f) is on the path of a device that lists the generic
/// wearable compound stream only, never an ET5's.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_wearable_advanced_data_subscribe(
    device: *mut Device,
    callback: *const c_void,
    _user_data: *mut c_void,
) -> Status {
    let export = "tobii_wearable_advanced_data_subscribe";
    refused(export, device, callback.is_null())
}

/// The unsubscribe of `tobii_wearable_advanced_data_subscribe`:
/// `TOBII_ERROR_NOT_SUPPORTED` once the device checks out, never
/// `TOBII_ERROR_NOT_SUBSCRIBED` (0x1801524c0, through 0x18015b340).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_wearable_advanced_data_unsubscribe(device: *mut Device) -> Status {
    refused("tobii_wearable_advanced_data_unsubscribe", device, false)
}

/// The lens configuration: `TOBII_ERROR_NOT_SUPPORTED` once the device and
/// `lens_config` check out, with nothing written. The DLL (0x180152410)
/// reads PRP property 0xa through its `tobii_property_get` (0x18015d300),
/// which answers `TOBII_ERROR_NOT_SUPPORTED` before any request
/// (0x18015d450) when the device's readable properties lack it, as an ET5's
/// do.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_get_lens_configuration(
    device: *mut Device,
    lens_config: *mut c_void,
) -> Status {
    let export = "tobii_get_lens_configuration";
    refused(export, device, lens_config.is_null())
}

/// Write the lens configuration: `TOBII_ERROR_NOT_SUPPORTED` once the device
/// and `lens_config` check out, with nothing read. The DLL (0x1801522e0)
/// checks `lens_config` before the device, copies it, and sets PRP property
/// 0xa through its `tobii_property_set` (0x18015c150), which checks the
/// callback flag and answers `TOBII_ERROR_NOT_SUPPORTED` before any request
/// (0x18015c29a) when the device's writable properties lack it, as an ET5's
/// do (hence `tobii_lens_configuration_writable`'s answer).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_set_lens_configuration(
    device: *mut Device,
    lens_config: *const c_void,
) -> Status {
    let export = "tobii_set_lens_configuration";
    refused(export, device, lens_config.is_null())
}

/// The extended lens configuration 4.25 adds: `TOBII_ERROR_NOT_SUPPORTED`
/// once the device and `lens_config` check out, with nothing written, exactly
/// as [`tobii_get_lens_configuration`] answers for the basic one.
///
/// 4.25 keeps both: the basic trio stays on PRP property 0xa with its
/// 24-byte value, and this one reads property 0x17, whose value is 48 bytes —
/// four `float[3]` at 0x00/0x0c/0x18/0x24, the first two at the basic
/// struct's offsets. The basic getter does **not** forward to it. No licence
/// is consulted in either path; the gate is the device's readable and
/// writable property lists, and no device advertises the extended one — 4.25's
/// own tracker module has no handler for it at all.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_get_lens_configuration_extended(
    device: *mut Device,
    lens_config: *mut c_void,
) -> Status {
    let export = "tobii_get_lens_configuration_extended";
    refused(export, device, lens_config.is_null())
}

/// Write the extended lens configuration: `TOBII_ERROR_NOT_SUPPORTED` once
/// the device and `lens_config` check out, with nothing read, as
/// [`tobii_set_lens_configuration`] answers for the basic one. 4.25 reads the
/// caller's 48 bytes into its own frame before asking the device, and the
/// device's writable properties lack property 0x17.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_set_lens_configuration_extended(
    device: *mut Device,
    lens_config: *const c_void,
) -> Status {
    let export = "tobii_set_lens_configuration_extended";
    refused(export, device, lens_config.is_null())
}

/// Whether the extended lens configuration can be written: as
/// [`tobii_lens_configuration_writable`], the answer is
/// `TOBII_LENS_CONFIGURATION_NOT_WRITABLE`, from the device's list of
/// writable properties, which has no `TOBII_ERROR_NOT_SUPPORTED` path.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `writable` must be null or
/// valid for writing one `tobii_lens_configuration_extended_writable_t`,
/// which is the same 32-bit enumeration as the basic one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_lens_configuration_extended_writable(
    device: *mut Device,
    writable: *mut u32,
) -> Status {
    // SAFETY: forwarded under the same contract; the two differ only in the
    // property the DLL looks for, and an ET5 lists neither.
    unsafe { tobii_lens_configuration_writable(device, writable) }
}

/// The foveated wearable gaze stream (`tobii_stream_t` 11):
/// `TOBII_ERROR_NOT_SUPPORTED` once the device and callback check out, as
/// the DLL (0x180152070) answers through its `tobii_stream_subscribe`
/// (0x18015cbf0), whose stream check refuses 11 outright on its own tracker
/// module (0x18015cd29).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_wearable_foveated_gaze_subscribe(
    device: *mut Device,
    callback: *const c_void,
    _user_data: *mut c_void,
) -> Status {
    let export = "tobii_wearable_foveated_gaze_subscribe";
    refused(export, device, callback.is_null())
}

/// The unsubscribe of `tobii_wearable_foveated_gaze_subscribe`:
/// `TOBII_ERROR_NOT_SUPPORTED` once the device checks out, never
/// `TOBII_ERROR_NOT_SUBSCRIBED` (0x180151fe0; its `tobii_stream_unsubscribe`
/// checks the stream first, 0x18015cfa0).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_wearable_foveated_gaze_unsubscribe(device: *mut Device) -> Status {
    refused("tobii_wearable_foveated_gaze_unsubscribe", device, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tobii_device_destroy;
    use crate::device::tests::{counting_device, destroy_untouched};
    use crate::status::{TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_NOT_SUPPORTED};
    use std::ptr;

    #[test]
    fn the_lens_configuration_is_never_writable() {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(0, vec![])));
        let mut w = 0xdead_beef_u32;
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `w` is a live local.
        unsafe {
            assert_eq!(tobii_lens_configuration_writable(d, &raw mut w), 0);
            assert_eq!(w, TOBII_LENS_CONFIGURATION_NOT_WRITABLE);
            assert_eq!(
                tobii_lens_configuration_writable(d, ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            w = 0xdead_beef;
            assert_eq!(
                tobii_lens_configuration_writable(ptr::null_mut(), &raw mut w),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(w, 0xdead_beef, "nothing written");
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    type Subscribe = extern "C" fn(*mut Device, *const c_void, *mut c_void) -> Status;
    type Unsubscribe = extern "C" fn(*mut Device) -> Status;

    /// Each wearable stream's subscribe and unsubscribe.
    const STREAMS: [(&str, Subscribe, Unsubscribe); 3] = [
        (
            "consumer",
            tobii_wearable_consumer_data_subscribe,
            tobii_wearable_consumer_data_unsubscribe,
        ),
        (
            "advanced",
            tobii_wearable_advanced_data_subscribe,
            tobii_wearable_advanced_data_unsubscribe,
        ),
        (
            "foveated",
            tobii_wearable_foveated_gaze_subscribe,
            tobii_wearable_foveated_gaze_unsubscribe,
        ),
    ];

    /// A callback the refused subscribes are given and never call.
    fn never_called() -> *const c_void {
        ptr::NonNull::<c_void>::dangling().as_ptr().cast_const()
    }

    /// A lens configuration (six floats) no call may write.
    const LENS: [u8; 24] = [0xa5; 24];

    /// The daemon counts every frame: none reaches it. An unsubscribe is
    /// refused, never `TOBII_ERROR_NOT_SUBSCRIBED`, as in the DLL.
    #[test]
    fn wearable_entry_points_are_not_supported_without_asking_the_daemon() {
        let (d, frames) = counting_device();
        let (refused, invalid) = (TOBII_ERROR_NOT_SUPPORTED, TOBII_ERROR_INVALID_PARAMETER);
        let n = ptr::null_mut();
        for (stream, subscribe, unsubscribe) in STREAMS {
            let got = [
                unsubscribe(d),
                subscribe(d, never_called(), n),
                subscribe(d, ptr::null(), n),
                subscribe(n.cast(), never_called(), n),
                subscribe(n.cast(), ptr::null(), n),
                unsubscribe(d),
                unsubscribe(n.cast()),
            ];
            let want = [
                refused, refused, invalid, invalid, invalid, refused, invalid,
            ];
            assert_eq!(got, want, "{stream}");
        }
        let mut lens = LENS;
        let out = lens.as_mut_ptr().cast::<c_void>();
        let got = [
            tobii_get_lens_configuration(d, out),
            tobii_get_lens_configuration(d, n),
            tobii_get_lens_configuration(n.cast(), out),
            tobii_set_lens_configuration(d, out.cast_const()),
            tobii_set_lens_configuration(d, ptr::null()),
            tobii_set_lens_configuration(n.cast(), out.cast_const()),
        ];
        assert_eq!(got, [refused, invalid, invalid, refused, invalid, invalid]);
        assert_eq!(lens, LENS, "nothing written");
        destroy_untouched(d, &frames);
    }

    /// Inside a callback every call is refused before its arguments are
    /// read, as wherever a device handle is taken. The DLL checks the nulls
    /// first, so for those it would answer `TOBII_ERROR_INVALID_PARAMETER`.
    #[test]
    fn wearable_entry_points_are_refused_inside_a_callback() {
        let (d, frames) = counting_device();
        let n = ptr::null_mut();
        let mut lens = LENS;
        let out = lens.as_mut_ptr().cast::<c_void>();
        let mut got = Vec::new();
        crate::device::call(|| {
            for (_, subscribe, unsubscribe) in STREAMS {
                got.extend([
                    subscribe(d, never_called(), n),
                    subscribe(n.cast(), ptr::null(), n),
                    unsubscribe(d),
                    unsubscribe(n.cast()),
                ]);
            }
            got.extend([
                tobii_get_lens_configuration(d, out),
                tobii_get_lens_configuration(n.cast(), n),
                tobii_set_lens_configuration(d, out.cast_const()),
                tobii_set_lens_configuration(n.cast(), ptr::null()),
            ]);
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 16]);
        assert_eq!(lens, LENS, "nothing written");
        // The callback guard is down again.
        destroy_untouched(d, &frames);
    }
}
