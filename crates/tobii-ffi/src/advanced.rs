//! `tobii_advanced.h`: per-eye gaze data. The digital sync port and the face
//! types check their arguments and then answer `TOBII_ERROR_NOT_SUPPORTED`:
//! for the sync port that is the DLL's answer for an ET5 it drives itself;
//! for the face types the DLL asks the tracker or checks a licence first, and
//! the answer is inferred (see each).

use std::ffi::{c_char, c_void};

use crate::device::Device;
use crate::status::Status;
use crate::streams::{subscribe, unsubscribe};
use crate::stub::refused;
use crate::types::GazeDataFn;

/// Per-eye gaze data: origins, gaze points and eyeball centres in the tracker
/// frame, normalised positions and display gaze points, and pupil diameters,
/// valid while the eye's status is below 2, as in the Stream Engine (the ET5
/// sends 0 for a tracked eye, 4 for a lost one).
///
/// # Safety
/// As the subscribe functions in `tobii_streams.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_gaze_data_subscribe(
    device: *mut Device,
    callback: Option<GazeDataFn>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { subscribe(device, |c| &mut c.gaze_data, callback, user_data) }
}

/// Undo `tobii_gaze_data_subscribe`.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_gaze_data_unsubscribe(device: *mut Device) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { unsubscribe(device, |c| &mut c.gaze_data) }
}

/// The digital sync port: `TOBII_ERROR_NOT_SUPPORTED` once the device
/// checks out, whatever the callback, as the DLL (0x180146200) answers for an
/// ET5: after a null device and the callback flag, and before its lock and
/// licence check, it refuses a device run by no tracker module of its own or
/// one whose sync-port state (`[module+0xa84c]`) is 2 (0x18014625a). That is
/// set at connect for a tracker whose serial does not start with "TX300"
/// (0x18016de63..0x18016de83), and lifted only for a stream catalogue (1200)
/// holding stream 0x507 (0x180001c86), which the ET5's does not. The DLL
/// never checks the callback for null; neither does this. It is never read,
/// so the function is safe.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_digital_syncport_subscribe(
    device: *mut Device,
    _callback: *const c_void,
    _user_data: *mut c_void,
) -> Status {
    refused("tobii_digital_syncport_subscribe", device, false)
}

/// The unsubscribe of `tobii_digital_syncport_subscribe`:
/// `TOBII_ERROR_NOT_SUPPORTED` once the device checks out, never
/// `TOBII_ERROR_NOT_SUBSCRIBED`, as in the DLL (0x180145fd0), which refuses
/// the sync port by the same test (0x18014601b) before it looks for a
/// subscription.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_digital_syncport_unsubscribe(device: *mut Device) -> Status {
    refused("tobii_digital_syncport_unsubscribe", device, false)
}

/// The face types the tracker knows: `TOBII_ERROR_NOT_SUPPORTED` once the
/// device and receiver check out, with the receiver never called (inferred).
///
/// The DLL (0x180145230) checks a null device, then a null receiver, then the
/// callback flag, and on its own tracker module asks the tracker (command
/// 3320, 0x18019e424) without consulting the tracker's capabilities. The
/// ET5's answer to 3320 was never captured; its capability answer (1350)
/// lists no face types (id 4 "false"), so the tracker presumably refuses,
/// which the module would map to `TOBII_ERROR_NOT_SUPPORTED` for a
/// not-supported error (table 0x180179118). Nothing is sent to the daemon.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_enumerate_face_types(
    device: *mut Device,
    receiver: *const c_void,
    _user_data: *mut c_void,
) -> Status {
    refused("tobii_enumerate_face_types", device, receiver.is_null())
}

/// Set the face type: `TOBII_ERROR_NOT_SUPPORTED` once the device and
/// `face_type` check out, with nothing read from it.
///
/// The DLL (0x180145040) checks a null device, then a null `face_type`, then
/// the callback flag, and on its own tracker module answers
/// `TOBII_ERROR_INSUFFICIENT_LICENSE` below the config feature group
/// (0x180179d5a), so for the consumer group this library reports. libtobii
/// gates nothing on a licence, so it answers what the tracker itself would
/// past that check, where the DLL sends command 3310 (0x1801901e0) with no
/// capability check: never captured, and inferred `TOBII_ERROR_NOT_SUPPORTED`
/// from the ET5's capability answer (1350 id 4 "false"). The string is never
/// read, so the function is safe.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_set_face_type(device: *mut Device, face_type: *const c_char) -> Status {
    refused("tobii_set_face_type", device, face_type.is_null())
}

/// The face type in use: `TOBII_ERROR_NOT_SUPPORTED` once the device and
/// `face_type` check out, with nothing written (inferred).
///
/// The DLL (0x180144e50) checks a null device, then a null `face_type`, then
/// the callback flag, and on its own tracker module asks the tracker
/// (command 3300, 0x180195805), copying at most 63 characters into
/// `face_type` only on success. The ET5's answer to 3300 was never captured;
/// see `tobii_enumerate_face_types` for why it is taken to be a refusal.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_get_face_type(device: *mut Device, face_type: *mut c_char) -> Status {
    refused("tobii_get_face_type", device, face_type.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::{counting_device, destroy_untouched};
    use crate::status::{
        TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NOT_SUPPORTED,
    };
    use std::ptr;

    /// A callback or receiver the refused calls are given and never call.
    fn never_called() -> *const c_void {
        ptr::NonNull::<c_void>::dangling().as_ptr().cast_const()
    }

    /// A face type (`char[64]`) no call may write.
    const FACE_TYPE: [c_char; 64] = [0x5a; 64];

    /// The daemon counts every frame: none reaches it. The sync port's
    /// callback is not checked, as in the DLL, and its unsubscribe is
    /// refused, never `TOBII_ERROR_NOT_SUBSCRIBED`.
    #[test]
    fn the_sync_port_and_face_types_are_not_supported_without_asking_the_daemon() {
        let (d, frames) = counting_device();
        let (refused, invalid) = (TOBII_ERROR_NOT_SUPPORTED, TOBII_ERROR_INVALID_PARAMETER);
        let n = ptr::null_mut();
        let mut face_type = FACE_TYPE;
        let out = face_type.as_mut_ptr();
        let got = [
            tobii_digital_syncport_unsubscribe(d),
            tobii_digital_syncport_subscribe(d, never_called(), n),
            tobii_digital_syncport_subscribe(d, ptr::null(), n),
            tobii_digital_syncport_subscribe(n.cast(), never_called(), n),
            tobii_digital_syncport_unsubscribe(n.cast()),
            tobii_enumerate_face_types(d, never_called(), n),
            tobii_enumerate_face_types(d, ptr::null(), n),
            tobii_enumerate_face_types(n.cast(), never_called(), n),
            tobii_set_face_type(d, out.cast_const()),
            tobii_set_face_type(d, ptr::null()),
            tobii_set_face_type(n.cast(), out.cast_const()),
            tobii_get_face_type(d, out),
            tobii_get_face_type(d, ptr::null_mut()),
            tobii_get_face_type(n.cast(), out),
        ];
        let want = [
            refused, refused, refused, invalid, invalid, refused, invalid, invalid, refused,
            invalid, invalid, refused, invalid, invalid,
        ];
        assert_eq!(got, want);
        assert_eq!(face_type, FACE_TYPE, "nothing written");
        destroy_untouched(d, &frames);
    }

    /// Inside a callback every call is refused before its arguments are
    /// read, as wherever a device handle is taken. The DLL checks the nulls
    /// first, so for those it would answer `TOBII_ERROR_INVALID_PARAMETER`.
    #[test]
    fn the_sync_port_and_face_types_are_refused_inside_a_callback() {
        let (d, frames) = counting_device();
        let n = ptr::null_mut();
        let mut face_type = FACE_TYPE;
        let out = face_type.as_mut_ptr();
        let mut got = Vec::new();
        crate::device::call(|| {
            got.extend([
                tobii_digital_syncport_subscribe(d, never_called(), n),
                tobii_digital_syncport_subscribe(n.cast(), ptr::null(), n),
                tobii_digital_syncport_unsubscribe(d),
                tobii_digital_syncport_unsubscribe(n.cast()),
                tobii_enumerate_face_types(d, never_called(), n),
                tobii_enumerate_face_types(n.cast(), ptr::null(), n),
                tobii_set_face_type(d, out.cast_const()),
                tobii_set_face_type(n.cast(), ptr::null()),
                tobii_get_face_type(d, out),
                tobii_get_face_type(n.cast(), ptr::null_mut()),
            ]);
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 10]);
        assert_eq!(face_type, FACE_TYPE, "nothing written");
        // The callback guard is down again.
        destroy_untouched(d, &frames);
    }
}
