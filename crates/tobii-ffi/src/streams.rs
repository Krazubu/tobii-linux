//! `tobii_streams.h`: the sample streams.
//!
//! Every subscribe entry point has the same contract: `device` must be null
//! or a live handle from `tobii_device_create` that is not destroyed before
//! the call returns; `callback` must be null (rejected) or sound to invoke
//! with a valid pointer to its sample type and `user_data` on any thread that
//! calls `tobii_device_process_callbacks` on the device, must not re-enter
//! this library (an implemented call that takes a device, one that creates a
//! device, `tobii_calibration_parse` or `tobii_api_destroy` is refused with
//! `TOBII_ERROR_CALLBACK_IN_PROGRESS`; see the crate documentation), and
//! `user_data` must stay valid until the stream is unsubscribed or the device
//! destroyed (until the call returns, if it fails). Callbacks run on the
//! thread that calls `tobii_device_process_callbacks`, one at a time per
//! device. A subscribe and an unsubscribe each wait for a callback of the
//! device that another thread is running. Once an unsubscribe returns, its
//! callback is not running on any thread, and none calls it again. A
//! subscribe may have its callback called, on a thread processing the
//! device, before it returns, even if it then fails; once it has returned an
//! error, none calls it again. The DLL stores a callback only once the
//! tracker has taken the subscription (0x1801537cc..0x1801537de), so a
//! failed subscribe's callback never runs there.

use std::ffi::c_void;

use crate::device::{Callbacks, Device, Slot, device_ref};
use crate::status::Status;
use crate::types::{
    EyePairFn, GazePointFn, HeadPoseFn, NotificationsFn, PresenceFn, PresenceSampleFn,
};

/// Subscribe `callback` into `slot` of the device behind `device`.
///
/// # Safety
/// See the module documentation.
pub(crate) unsafe fn subscribe<F: Copy>(
    device: *mut Device,
    slot: fn(&mut Callbacks) -> &mut Slot<F>,
    callback: Option<F>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    match unsafe { device_ref(device) } {
        Ok(d) => d.subscribe(slot, callback, user_data),
        Err(status) => status,
    }
}

/// Unsubscribe `slot` of the device behind `device`.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
pub(crate) unsafe fn unsubscribe<F: Copy>(
    device: *mut Device,
    slot: fn(&mut Callbacks) -> &mut Slot<F>,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    match unsafe { device_ref(device) } {
        Ok(d) => d.unsubscribe(slot),
        Err(status) => status,
    }
}

/// Define a subscribe/unsubscribe pair over one callback slot.
macro_rules! stream_pair {
    ($(
        $(#[$doc:meta])*
        $sub:ident / $unsub:ident: $slot:ident, $fn_ty:ty;
    )+) => { $(
        $(#[$doc])*
        ///
        /// # Safety
        /// See the module documentation.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $sub(device: *mut Device, callback: Option<$fn_ty>, user_data: *mut c_void) -> Status {
            // SAFETY: forwarded under the same contract.
            unsafe { subscribe(device, |c| &mut c.$slot, callback, user_data) }
        }

        #[doc = concat!("Undo `", stringify!($sub), "`.")]
        ///
        /// # Safety
        /// `device` must be null or a live handle that is not destroyed
        /// before the call returns.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $unsub(device: *mut Device) -> Status {
            // SAFETY: forwarded under the same contract.
            unsafe { unsubscribe(device, |c| &mut c.$slot) }
        }
    )+ };
}

stream_pair! {
    /// Gaze point, normalised display coordinates (the device's filtered
    /// combined gaze, unclamped, as the Stream Engine reports it).
    tobii_gaze_point_subscribe / tobii_gaze_point_unsubscribe: gaze, GazePointFn;
    /// Per-eye gaze origin in the display frame, mm.
    tobii_gaze_origin_subscribe / tobii_gaze_origin_unsubscribe: gaze_origin, EyePairFn;
    /// The eye position: the tracker's own Eye 3D Relative Position, each
    /// eye's place in the camera image and its distance from the tracker,
    /// normalised to about `0..1` per axis (see `tobii_streams.h`).
    tobii_eye_position_normalized_subscribe / tobii_eye_position_normalized_unsubscribe: eye_position, EyePairFn;
    /// User presence, reported once on subscribe and then on change.
    tobii_user_presence_subscribe / tobii_user_presence_unsubscribe: presence, PresenceFn;
    /// The Stream Engine's head pose, from the tracker's IR images (see
    /// [`HeadPose`](crate::types::HeadPose)): absolute, in the display
    /// frame, one for every image tobiid processes, each value with its
    /// validity. It is tobiid's `HEAD_POSE` stream: a tobiid from before it
    /// acks the subscription and sends nothing, which a device logs at WARN
    /// once it has gone a while without one (see `device::HeadPoseWatch`).
    tobii_head_pose_subscribe / tobii_head_pose_unsubscribe: head, HeadPoseFn;
    /// Device notifications: display-area, calibration and pause changes,
    /// and the tracker's fault and warning lists.
    tobii_notifications_subscribe / tobii_notifications_unsubscribe: notifications, NotificationsFn;
    /// User presence as 4.25 serves it: the same status and timestamp
    /// `tobii_user_presence_subscribe` delivers, in a `tobii_presence_t`
    /// instead of two arguments. 4.25 reads it from PRP property 0x14 rather
    /// than a stream, and the current reference marks it deprecated; here
    /// both are fed from the daemon's one presence stream, so a subscriber
    /// to either sees the same changes.
    tobii_presence_subscribe / tobii_presence_unsubscribe: presence_sample, PresenceSampleFn;
    /// The responsive combined gaze point (`tobii_stream_t` 12, the stream
    /// 4.25 adds): the tracker's own "Remote Gaze Point Responsive
    /// Combined", stream 0x83 key `0x20` with validity `0x1f`, which reaches
    /// this library in the daemon's `GAZE_RAW` message as the combined point
    /// — the less filtered point the ET5 sends beside the one
    /// `tobii_gaze_point_subscribe` delivers. It rides on the same daemon
    /// stream as raw gaze, so subscribing to it alone starts that stream.
    tobii_responsive_gaze_point_subscribe / tobii_responsive_gaze_point_unsubscribe: responsive_gaze, GazePointFn;
    /// The user position guide: the tracker's own User 3D Position Guide,
    /// each eye's place in the camera image and a depth that rises with its
    /// distance, with a validity of its own (see `tobii_streams.h`). A
    /// tobiid from before it sends none, and the guide is then the eye
    /// positions.
    tobii_user_position_guide_subscribe / tobii_user_position_guide_unsubscribe: user_position_guide, EyePairFn;
}
