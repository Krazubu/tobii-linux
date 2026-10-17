//! The 73 exports of `tobii_stream_engine.dll` 4.1.0.3 that its documentation
//! does not cover (all but `tobii_calibration_stimulus_points_get`, which
//! lives with calibration). Their argument counts come from the DLL (see
//! `tools/abi`); their types are best-effort, which is safe because only the
//! field-of-use, image, raw gaze, foveated rendering gaze point,
//! internal-stream, internal-capability, timesync, stream-type,
//! stream-column, pause, hardware-configuration and internal device-info
//! functions below read their arguments, and
//! `tobii_send_statistics` its two strings, typed as the DLL takes them; the
//! subscribes and unsubscribes of the internal streams the DLL refuses, the
//! face id calls and `tobii_set_display_info` only compare theirs with null,
//! and `tobii_set_fw_upgrade_allowed` reads none.

use std::ffi::{c_char, c_void};

use tobii_ipc::request::{
    self, decode_hardware_configuration, decode_stream_columns, decode_stream_types,
    decode_timesync, encode_u32, kind,
};

use crate::api::write_supported;
use crate::calibration::request;
use crate::device::{Device, device_ref, in_callback};
use crate::status::{
    Status, TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_CONNECTION_FAILED,
    TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NO_ERROR, TOBII_ERROR_NOT_AVAILABLE,
    TOBII_ERROR_NOT_SUPPORTED,
};
use crate::streams::{subscribe, unsubscribe};
use crate::stub::{not_supported, refused};
use crate::timeouts;
use crate::types::{
    DeviceInfoInternal, FieldOfUse, FieldOfUseFn, GazePointFn, GazeRawFn, HardwareConfiguration,
    HardwareConfigurationEntry, ImageFn, StreamType, StreamTypeColumn, StreamTypeColumnReceiver,
    StreamTypeReceiver, TimesyncData, copy_c_bytes, copy_c_string,
};

/// The field of use the device was created with.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns; `field_of_use` must be null or valid for writing one
/// `tobii_field_of_use_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_get_field_of_use(
    device: *mut Device,
    field_of_use: *mut FieldOfUse,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    if field_of_use.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    // SAFETY: non-null, and the caller guarantees it is writable.
    unsafe { field_of_use.write(d.field_of_use) };
    TOBII_ERROR_NO_ERROR
}

/// Register a field-of-use callback. It is never called: the field of use is
/// fixed when the device is created.
///
/// # Safety
/// As the subscribe functions in `tobii_streams.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_field_of_use_subscribe(
    device: *mut Device,
    callback: Option<FieldOfUseFn>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { subscribe(device, |c| &mut c.field_of_use, callback, user_data) }
}

/// Undo `tobii_field_of_use_subscribe`.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_field_of_use_unsubscribe(device: *mut Device) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { unsubscribe(device, |c| &mut c.field_of_use) }
}

/// The tracker's IR camera: 280x280, 8 bits per pixel, ~33 Hz (~2.6 MB/s).
/// `tobii_image_t.data` is valid only during the callback.
///
/// # Safety
/// As the subscribe functions in `tobii_streams.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_image_subscribe(
    device: *mut Device,
    callback: Option<ImageFn>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { subscribe(device, |c| &mut c.image, callback, user_data) }
}

/// Undo `tobii_image_subscribe`.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_image_unsubscribe(device: *mut Device) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { unsubscribe(device, |c| &mut c.image) }
}

/// The Stream Engine's raw gaze: its own record of each gaze frame,
/// `tobii_gaze_raw_t` (see `tobii_internal.h`), ~33 Hz, stamped with the
/// tracker's clock. Every value is passed as the tracker sent it; a key's
/// `tobii_validity_t` flag says only that the frame had the key.
///
/// The DLL serves it only from its in-process tracker module, which it runs
/// for any URL, libtobii's `tobii-ffi://` included, but `tobii-prp://` and
/// `tprp-tcp://`, and only with the internal feature group:
/// `TOBII_ERROR_INSUFFICIENT_LICENSE` below it (0x180175d15), and
/// `TOBII_ERROR_NOT_SUPPORTED` behind the Tobii service, where that module
/// does not exist (0x18014e3e8). No licence is checked here (see
/// `licensing`). A daemon older than this library acks the subscription and
/// never sends the stream.
///
/// # Safety
/// As the subscribe functions in `tobii_streams.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_gaze_raw_subscribe(
    device: *mut Device,
    callback: Option<GazeRawFn>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { subscribe(device, |c| &mut c.gaze_raw, callback, user_data) }
}

/// Undo `tobii_gaze_raw_subscribe`.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_gaze_raw_unsubscribe(device: *mut Device) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { unsubscribe(device, |c| &mut c.gaze_raw) }
}

/// Internal streams this library delivers: the IR image (id 0) only.
///
/// The DLL's internal stream ids are 0 image, 1 clean IR, 2 custom, 3 and 4
/// low-frequency head rotation and position, 5 multiple faces, 6 image
/// collection, 7 wearable limited image and 8 secondary camera image. The
/// names of 3..8 come from the exports that subscribe to them; those of 0..2
/// are inferred.
const fn internal_stream_supported(stream: u32) -> bool {
    stream == 0
}

/// Whether an internal stream is available: the IR image only.
///
/// This matches what libtobii delivers (`tobii_image_subscribe` works; the
/// clean IR, custom and image-collection streams are stubs). For 3, 4, 5, 7
/// and 8 it is also the DLL's answer on its TTP path, for any tracker it
/// drives itself: its check (0x180157860, called at 0x18014cc59) has no case
/// for them, and their subscribes refuse them the same way (see
/// `refused_internal_subscribe`). For an ET5 the DLL would support 0, 2 and 6
/// on that path. On its PRP path, behind the Tobii service, it never
/// supports 0 or 1 and answers 2..8 from the streams the service lists,
/// never captured for an ET5. As in the DLL, an unknown id is reported
/// unsupported, not an error, and an id above `i32::MAX` is an invalid
/// parameter.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `supported` must be null or
/// valid for writing one `tobii_supported_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_internal_stream_supported(
    device: *mut Device,
    stream: u32,
    supported: *mut u32,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { write_supported(device, stream, supported, internal_stream_supported) }
}

/// A subscribe to an internal stream the DLL refuses for a tracker it drives
/// itself: 3 and 4 low-frequency head rotation and position, 5 multiple
/// faces position, 7 wearable limited image and 8 secondary camera image.
///
/// Each such export passes its stream's id to one helper of the DLL
/// (0x18015bbf0, `tobii_internal_stream_subscribe` in its log). Its support
/// check (0x180157860) has no case for these ids when the DLL runs its own
/// tracker module, as it does for any URL but `tobii-prp://` and
/// `tprp-tcp://`, so the helper answers `TOBII_ERROR_NOT_SUPPORTED` before it
/// registers anything (0x18015bd29); the module's own subscribes for them
/// answer `PLATMOD_ERROR_NOT_SUPPORTED` whatever they are given. Behind the
/// Tobii service the answer depends on the streams the service lists, never
/// captured for an ET5. So nothing is sent to the daemon, and libtobii's head
/// pose is not re-published as the low-frequency streams, whose units, frame
/// and rate would all be invented.
///
/// A call from inside a callback is `TOBII_ERROR_CALLBACK_IN_PROGRESS`, then a
/// null device or callback is `TOBII_ERROR_INVALID_PARAMETER`, in that order:
/// the DLL checks both nulls before the callback. Unlike the DLL, which logs
/// every refusal but a null device's at ERROR, nothing reaches the
/// application's logger. The device is only compared with null, never
/// borrowed.
fn refused_internal_subscribe(
    export: &'static str,
    stream: u32,
    device: *mut Device,
    callback: *const c_void,
) -> Status {
    if in_callback() {
        return TOBII_ERROR_CALLBACK_IN_PROGRESS;
    }
    if device.is_null() || callback.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    tracing::trace!(export, stream, "internal stream not supported");
    TOBII_ERROR_NOT_SUPPORTED
}

/// The unsubscribe of an internal stream [`refused_internal_subscribe`]
/// refuses. The DLL's helper (0x18015bee0) runs the same support check before
/// it looks for a subscription (0x18015bf2d), so the answer is
/// `TOBII_ERROR_NOT_SUPPORTED`, never `TOBII_ERROR_NOT_SUBSCRIBED`; the
/// checks before it are the subscribe's, less the callback.
fn refused_internal_unsubscribe(export: &'static str, stream: u32, device: *mut Device) -> Status {
    if in_callback() {
        return TOBII_ERROR_CALLBACK_IN_PROGRESS;
    }
    if device.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    tracing::trace!(export, stream, "internal stream not supported");
    TOBII_ERROR_NOT_SUPPORTED
}

/// Define the subscribe and unsubscribe of internal streams the DLL refuses
/// for a tracker it drives itself (see `refused_internal_subscribe`).
macro_rules! refused_internal_streams {
    ($(
        $(#[$doc:meta])*
        $sub:ident / $unsub:ident: $stream:literal;
    )+) => { $(
        $(#[$doc])*
        ///
        /// `TOBII_ERROR_NOT_SUPPORTED` once the device and callback check out,
        /// as the DLL answers for a tracker it drives itself, with nothing sent
        /// to the daemon (see `refused_internal_subscribe`). The pointers are
        /// only compared with null, so the function is safe.
        #[unsafe(no_mangle)]
        pub extern "C" fn $sub(
            device: *mut Device,
            callback: *const c_void,
            _user_data: *mut c_void,
        ) -> Status {
            refused_internal_subscribe(stringify!($sub), $stream, device, callback)
        }

        #[doc = concat!(
            "The unsubscribe of `", stringify!($sub), "`: `TOBII_ERROR_NOT_SUPPORTED` ",
            "once the device checks out, as in the DLL, whose support check comes ",
            "before its subscription lookup (see `refused_internal_unsubscribe`)."
        )]
        #[unsafe(no_mangle)]
        pub extern "C" fn $unsub(device: *mut Device) -> Status {
            refused_internal_unsubscribe(stringify!($unsub), $stream, device)
        }
    )+ };
}

refused_internal_streams! {
    /// Internal stream 3, low-frequency head rotation (the DLL's PRP stream 9,
    /// `LOW_FREQUENCY_HEAD_ROTATION`). What the DLL would deliver behind the
    /// Tobii service is described in `tobii_internal.h`.
    tobii_low_frequency_head_rotation_subscribe / tobii_low_frequency_head_rotation_unsubscribe: 3;
    /// Internal stream 4, low-frequency head position (the DLL's PRP stream 8,
    /// `LOW_FREQUENCY_HEAD_POSITION`). What the DLL would deliver behind the
    /// Tobii service is described in `tobii_internal.h`.
    tobii_low_frequency_head_position_subscribe / tobii_low_frequency_head_position_unsubscribe: 4;
    /// Internal stream 5, multiple faces position (the DLL's PRP stream 10,
    /// `MULTIPLE_FACES_POSITION`).
    tobii_multiple_faces_position_subscribe / tobii_multiple_faces_position_unsubscribe: 5;
    /// Internal stream 7, wearable limited image (the DLL's PRP stream 11,
    /// `WEARABLE_LIMITED_IMAGE`).
    tobii_wearable_limited_image_subscribe / tobii_wearable_limited_image_unsubscribe: 7;
    /// Internal stream 8, secondary camera image (the DLL's PRP stream 0x17,
    /// `SECONDARY_CAMERA_IMAGE`).
    tobii_secondary_camera_image_subscribe / tobii_secondary_camera_image_unsubscribe: 8;
}

/// Internal capabilities this library provides: eyeball centres (id 0) only.
///
/// The DLL's ids, from what its jump table at 0x18014d268 leads to: 0
/// eyeball center, 1 diagnostic images (command 0x15), 2 remote wake
/// (settable property 3), 3 power save (settable property 2), 4 face id
/// (properties 13 and 14, commands 0x1a and 0x1b) and 5 logs (command
/// 0x19). The name of 0 is the DLL's; those of 1..5 are inferred from what
/// they look for.
const fn internal_capability_supported(capability: u32) -> bool {
    capability == 0
}

/// Whether an internal capability is available: eyeball centres only.
///
/// This matches what libtobii delivers (`tobii_gaze_data_t` carries each
/// eye's eyeball centre; diagnostic images, remote wake, power save and logs
/// are stubs, and face id is refused as the DLL refuses it for an ET5), not
/// the DLL. For an ET5 the DLL would support 0 on
/// its TTP path (inferred: it asks for gaze columns 0x17 and 0x18, the keys
/// of the eyeball centres in every ET5 gaze frame) and answer
/// `TOBII_ERROR_NOT_SUPPORTED` for 0 on its PRP path, the one behind the
/// Tobii service; it answers 1..5 from device lists never captured for an
/// ET5. As in the DLL, an id above 5 is reported unsupported, not an error,
/// and a negative id is an invalid parameter.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `supported` must be null or
/// valid for writing one `tobii_supported_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_internal_capability_supported(
    device: *mut Device,
    capability: i32,
    supported: *mut u32,
) -> Status {
    // A negative id is one above `i32::MAX` here, which `write_supported`
    // refuses once the device checks out.
    let capability = capability.cast_unsigned();
    // SAFETY: forwarded under the same contract.
    unsafe { write_supported(device, capability, supported, internal_capability_supported) }
}

/// A fresh tracker/host clock pair: the tracker clock read `tracker_us` at
/// some host time between `system_start_us` and `system_end_us`.
///
/// The daemon answers from the first gaze frame it receives after the
/// request (starting the tracker if needed): `tracker_us` is the frame's
/// device timestamp and the bracket is the 30 ms before the daemon read it.
/// The DLL instead times a round trip to its service, and its own offset
/// estimator skips pairs wider than 6 ms, though it still returns them. The
/// host clock is `tobii_system_clock`'s, `CLOCK_MONOTONIC` rather than the
/// DLL's QPC, though like it monotonic with an undefined epoch. The
/// callbacks' `timestamp_us` (and gaze data's `timestamp_system_us`) need no
/// pair: the daemon sends them on the host clock already; gaze data's
/// `timestamp_tracker_us` is the tracker time a pair maps. The tracker's
/// clock may restart when the tracker re-initialises, so a pair holds until
/// then only. Nothing is written unless the call succeeds;
/// `TOBII_ERROR_NOT_AVAILABLE` while the tracker is paused,
/// `TOBII_ERROR_CONNECTION_FAILED` when no tracker is plugged in.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `timesync` must be null or
/// valid for writing one `tobii_timesync_data_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_timesync(
    device: *mut Device,
    timesync: *mut TimesyncData,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    if timesync.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    match d
        .request(kind::TIMESYNC, &[], timeouts::TIMESYNC)
        .map(|p| decode_timesync(&p))
    {
        Ok(Some(t)) => {
            // The reply's order is start, device, end; the struct's is
            // start, end, tracker.
            let out = TimesyncData {
                system_start_us: t.host_start_us,
                system_end_us: t.host_end_us,
                tracker_us: t.device_us,
            };
            // SAFETY: non-null, and the caller guarantees it is writable.
            unsafe { timesync.write(out) };
            TOBII_ERROR_NO_ERROR
        }
        Ok(None) => d.malformed("timesync"),
        Err(status) => status,
    }
}

/// The Stream Engine's stream type for a device stream id: the DLL's three
/// tables composed (id to TTP code at 0x18017c784, TTP to tracker at
/// 0x180190ae8, tracker to Stream Engine at 0x180001e74). 0x509 is 0 in the
/// DLL's table, and so are the ids the ET5 lists that the DLL has no type
/// for (0x50e, 0x1771, 0x1772, 0x1774).
const fn se_stream_type(id: u32) -> i32 {
    match id {
        0x500 => 1,
        0x501 => 2,
        0x502 => 3,
        0x503 => 14,
        0x504 => 4,
        0x505 => 5,
        0x506 => 8,
        0x507 => 9,
        0x508 => 11,
        0x50a => 6,
        0x1770 => 7,
        _ => 0,
    }
}

/// A catalogue entry as the DLL hands it over: the device's stream id
/// becomes a Stream Engine type, and the strings are cut to 63 bytes.
fn stream_type_c(t: &request::StreamType) -> StreamType {
    let mut c = StreamType {
        type_: se_stream_type(t.id),
        value: t.value,
        name: [0; 64],
        text: [0; 64],
    };
    copy_c_string(&mut c.name, &t.name);
    copy_c_string(&mut c.text, &t.text);
    c
}

/// The tracker's stream catalogue: `receiver` is called once per stream,
/// in the tracker's order.
///
/// The daemon answers from the catalogue the tracker reported at its last
/// init (starting the tracker if needed), where the DLL asks the tracker on
/// every call. The list is the tracker's own, so it names streams libtobii
/// does not deliver, such as `image_collection` (internal stream 6 is
/// unsupported). The DLL answers `TOBII_ERROR_NOT_SUPPORTED` on its PRP path
/// and needs the internal feature group on its TTP path; libtobii has no
/// licence gate here. Every entry is built before the first call, so the
/// receiver may call back into this library, as the DLL allows.
///
/// `TOBII_ERROR_NOT_SUPPORTED` if the tracker reported no catalogue (the DLL
/// answers `TOBII_ERROR_NO_ERROR` with no calls) or the daemon is older than
/// this library; `TOBII_ERROR_TIMED_OUT` if no tracker has been seen.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `receiver` must be null or
/// sound to call with a `tobii_stream_type_t` (valid during the call only)
/// and `user_data`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_enumerate_stream_types(
    device: *mut Device,
    receiver: Option<StreamTypeReceiver>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    let Some(receiver) = receiver else {
        return TOBII_ERROR_INVALID_PARAMETER;
    };
    let entries: Vec<StreamType> = match d
        .request(kind::STREAM_TYPES, &[], timeouts::FACTS)
        .map(|p| decode_stream_types(&p))
    {
        Ok(Some(types)) => types.iter().map(stream_type_c).collect(),
        Ok(None) => return d.malformed("stream types"),
        Err(status) => return status,
    };
    for entry in &entries {
        // SAFETY: the caller guarantees `receiver` is sound to call like
        // this; `entry` outlives the call.
        unsafe { receiver(entry, user_data) };
    }
    TOBII_ERROR_NO_ERROR
}

/// The device stream id the DLL asks command 1210 about for a Stream Engine
/// stream type: its map from type to device id (0x18015e580, the switch at
/// 0x18019d018, then 0x1801801a0). It is not the inverse of
/// [`se_stream_type`]: 12 maps to 0x1770 as 7 does, and 14, the type 0x503
/// maps to there, maps to 0 here, as 0, 13 and every type above 14 do. 10
/// maps to 0 too, but is never asked about: the DLL refuses it before the
/// map (0x180176fac), and so does `tobii_enumerate_stream_type_columns`.
const fn columns_stream_id(stream_type: u32) -> u32 {
    match stream_type {
        1 => 0x500,
        2 => 0x501,
        3 => 0x502,
        4 => 0x504,
        5 => 0x505,
        6 => 0x50a,
        7 | 12 => 0x1770,
        8 => 0x506,
        9 => 0x507,
        11 => 0x508,
        _ => 0,
    }
}

/// Most columns the DLL hands over for one stream: it parses an answer's
/// records, 140 bytes each, into an arena with room for 73 (0x18017c1d8),
/// and a longer answer fails, `TOBII_ERROR_CONNECTION_FAILED`, before its
/// receiver is called.
const COLUMNS_MAX: usize = 73;

/// A stream's column as the DLL hands it over: the strings cut to 63
/// bytes, and every byte after each one's NUL zero (the DLL's record leaves
/// those as its stack had them, 0x180001eb0).
fn stream_type_column_c(c: &request::StreamColumn) -> StreamTypeColumn {
    let mut out = StreamTypeColumn {
        key: c.key,
        param_a: c.param_a,
        param_b: c.param_b,
        name: [0; 64],
        text: [0; 64],
    };
    copy_c_string(&mut out.name, &c.name);
    copy_c_string(&mut out.text, &c.text);
    out
}

/// The columns of one of the tracker's streams: `receiver` is called once
/// per column, in the tracker's order, with the key the stream's frames
/// carry it under, its name (`Left User 3D Position Guide` for gaze's key
/// 0x25), and two numbers and a string whose meaning is unknown (0, 1 and
/// an empty string in every column on the ET5).
///
/// `stream_type` is a Stream Engine stream type, as
/// `tobii_enumerate_stream_types` hands them out, which the DLL's map turns
/// into the device's stream id (see [`columns_stream_id`]); type 10 is
/// `TOBII_ERROR_NOT_SUPPORTED` with nothing asked, as in the DLL
/// (0x180176fac). The daemon answers from what the tracker answered the
/// command 1210 its last init sent for that stream (starting the tracker if
/// needed), where the DLL sends a 1210 at every call
/// (`tracker_enumerate_stream_type_columns`, 0x18019c9a0). The init asks
/// only about the streams its catalogue lists that a type maps to, so for a
/// stream the tracker does not list, and for the types that map to no
/// stream (0, and 13 and above), the daemon answers
/// `TOBII_ERROR_NOT_SUPPORTED`, where the DLL sends its 1210 all the same
/// and answers as the tracker does. For a stream whose 1210 the tracker
/// refused it answers `TOBII_ERROR_NOT_SUPPORTED` too, whatever the
/// refusal, where the DLL maps the tracker's error to one of its own
/// (0x180177324). An answer that lists no columns is
/// `TOBII_ERROR_NO_ERROR` with no call; one that lists more than 73 is
/// `TOBII_ERROR_CONNECTION_FAILED` with no call, as in the DLL
/// ([`COLUMNS_MAX`]).
///
/// The DLL (0x18014fa50) checks a null device and receiver before its
/// callback flag; libtobii checks the flag first, as for every entry point
/// that takes a handle. The DLL answers `TOBII_ERROR_NOT_SUPPORTED` on its
/// PRP path (0x18014fb27) and needs the internal feature group on its TTP
/// path (`TOBII_ERROR_INSUFFICIENT_LICENSE` below it, 0x180176f2a); libtobii
/// has no licence gate here. Every record is built before the first call,
/// and the receiver runs under no lock and no callback guard, so it may
/// call back into this library, as the DLL allows: it calls its receiver
/// once per record without its callback flag (0x18019cf50..0x18019cfe1,
/// through 0x180001eb0), though under its device's API mutex.
///
/// `TOBII_ERROR_NOT_SUPPORTED` too from a daemon older than this library;
/// `TOBII_ERROR_TIMED_OUT` if no tracker has been seen.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `receiver` must be null or
/// sound to call with a `tobii_stream_type_column_t` (valid during the call
/// only) and `user_data`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_enumerate_stream_type_columns(
    device: *mut Device,
    stream_type: u32,
    receiver: Option<StreamTypeColumnReceiver>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    let Some(receiver) = receiver else {
        return TOBII_ERROR_INVALID_PARAMETER;
    };
    // Refused before the DLL's map, so before its 1210 (0x180176fac).
    if stream_type == 10 {
        return TOBII_ERROR_NOT_SUPPORTED;
    }
    let id = columns_stream_id(stream_type);
    let columns = match d
        .request(kind::STREAM_COLUMNS, &encode_u32(id), timeouts::FACTS)
        .map(|p| decode_stream_columns(&p))
    {
        Ok(Some(columns)) => columns,
        Ok(None) => return d.malformed("stream columns"),
        Err(status) => return status,
    };
    if columns.len() > COLUMNS_MAX {
        return TOBII_ERROR_CONNECTION_FAILED;
    }
    let records: Vec<StreamTypeColumn> = columns.iter().map(stream_type_column_c).collect();
    for record in &records {
        // SAFETY: the caller guarantees `receiver` is sound to call like
        // this; `record` outlives the call.
        unsafe { receiver(record, user_data) };
    }
    TOBII_ERROR_NO_ERROR
}

/// Pause the tracker: it stops sending data until resumed.
///
/// The pause is one state for the tracker, shared by every client, as in the
/// DLL: the last call wins and any client may resume. Unlike the DLL, a
/// pause the tracker accepts shows at once in `TOBII_STATE_DEVICE_PAUSED`
/// and a `TOBII_NOTIFICATION_TYPE_DEVICE_PAUSED_STATE_CHANGED`
/// notification. A pause ends when the client that paused last disconnects
/// and whenever the tracker re-initialises. `TOBII_ERROR_CALIBRATION_BUSY`
/// while a calibration session runs.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_pause_device(device: *mut Device) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { request(device, kind::DEVICE_PAUSE, &[1], timeouts::DEVICE_PAUSE) }
}

/// Resume the tracker, whichever client paused it. A resume the tracker does
/// not answer still succeeds: the daemon re-opens a tracker that stays
/// silent, which resumes it.
///
/// # Safety
/// As `tobii_pause_device`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_resume_device(device: *mut Device) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { request(device, kind::DEVICE_PAUSE, &[0], timeouts::DEVICE_PAUSE) }
}

/// A hardware configuration entry with nothing set.
const NO_HARDWARE_ENTRY: HardwareConfigurationEntry = HardwareConfigurationEntry {
    id: 0,
    param_a: 0.0,
    param_b: 0.0,
    position_xyz: [0.0; 3],
    values: [0.0; 15],
    width: 0,
    height: 0,
    param_c: 0,
    coefficient_count: 0,
    coefficients: [0.0; 64],
    point_a_xyz: [0.0; 3],
    point_b_xyz: [0.0; 3],
    param_d: 0.0,
};

/// A count of slots, which is at most an array's length.
fn c_count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// The daemon's hardware configuration in the DLL's layout: every slot past
/// a count is zero, and the tracker's 32-bit words keep their bits, as the
/// DLL copies them. A mode outside 0..=2 is 0, as the DLL's service maps it.
#[allow(clippy::cast_possible_truncation)] // reason: the C fields are float (inferred); 16.16 values fit
fn hardware_configuration_c(h: &request::HardwareConfiguration) -> HardwareConfiguration {
    let mut c = HardwareConfiguration {
        entry_count: c_count(h.entries.len().min(2)),
        entries: [NO_HARDWARE_ENTRY; 2],
        point_count: c_count(h.points_mm.len().min(40)),
        points_xyz: [[0.0; 3]; 40],
        mode: match h.mode {
            mode @ 0..=2 => mode.cast_signed(),
            _ => 0,
        },
    };
    for (dst, e) in c.entries.iter_mut().zip(&h.entries) {
        let n = e.coefficients.len().min(dst.coefficients.len());
        dst.coefficients[..n].copy_from_slice(&e.coefficients[..n]);
        dst.coefficient_count = c_count(n);
        dst.id = e.id.cast_signed();
        dst.param_a = e.param_a as f32;
        dst.param_b = e.param_b as f32;
        dst.position_xyz = e.position_mm;
        dst.values = e.values;
        dst.width = e.width.cast_signed();
        dst.height = e.height.cast_signed();
        dst.param_c = e.param_c.cast_signed();
        dst.point_a_xyz = e.point_a_mm;
        dst.point_b_xyz = e.point_b_mm;
        dst.param_d = e.param_d;
    }
    for (dst, p) in c.points_xyz.iter_mut().zip(&h.points_mm) {
        *dst = *p;
    }
    c
}

/// The tracker's hardware configuration: provisional.
///
/// The DLL reads it from its service (PRP property 12), which has it from
/// the tracker's command 2120. That answer's layout is inferred from one
/// Windows capture, and on Linux the ET5 has so far answered 2120 with no
/// data, so this is `TOBII_ERROR_NOT_SUPPORTED` there, as the DLL answers
/// when a device does not list the property. When the daemon has one, the
/// whole struct is written, with every slot past a count zero (the DLL
/// leaves those as the caller had them); nothing is written unless the call
/// succeeds. The field names and units are ours: 16.16 values unscaled,
/// 32.32 values as lengths in mm.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `configuration` must be
/// null or valid for writing one `tobii_hardware_configuration_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_hardware_configuration_get(
    device: *mut Device,
    configuration: *mut HardwareConfiguration,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    if configuration.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    match d
        .request(kind::HARDWARE_CONFIGURATION, &[], timeouts::FACTS)
        .map(|p| decode_hardware_configuration(&p))
    {
        Ok(Some(h)) => {
            // SAFETY: non-null, and the caller guarantees it is writable.
            unsafe { configuration.write(hardware_configuration_c(&h)) };
            TOBII_ERROR_NO_ERROR
        }
        Ok(None) => d.malformed("hardware configuration"),
        Err(status) => status,
    }
}

/// Bytes in each string of `tobii_device_info_internal_t`, the NUL
/// included.
const INTERNAL_INFO_STRING: usize = 128;

/// The longest property string the DLL's tracker module keeps: its
/// `tracker_get_properties` (0x1801a22a0) copies each with `strncpy` of 0x78
/// bytes and ends it at the last of them (0x1801a2866..0x1801a287f for
/// property 3, 0x1801a2898..0x1801a28b1 for 4).
const PROPERTY_KEPT: usize = 0x77;

/// What of a tracker's property string reaches the DLL's caller: the bytes
/// before its first NUL, as C's string functions see it, and at most
/// [`PROPERTY_KEPT`] of them.
fn kept_property(s: &str) -> &[u8] {
    let bytes = s.as_bytes();
    let len = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    &bytes[..len.min(PROPERTY_KEPT)]
}

/// Properties 3 and 4 of the tracker's own property list (command 1330), in
/// two 128-byte strings: the platform type, `IS5LEYETRACKER5` on the ET5,
/// and an empty string, the ET5 sending no property 4.
///
/// The DLL (0x18014fe50) answers `TOBII_ERROR_INVALID_PARAMETER` for a null
/// device or output, then `TOBII_ERROR_CALLBACK_IN_PROGRESS` inside a
/// callback; libtobii checks the callback first, as for every entry point
/// that takes a handle. Under its API mutex the DLL then answers
/// `TOBII_ERROR_INSUFFICIENT_LICENSE` below the internal feature group
/// (0x18014ff16), so for a consumer ET5 without an internal licence key. No
/// licence is checked here (see `licensing`), so this answers what it would
/// past that check: both strings, each zero after its NUL as the DLL's
/// `strncpy` leaves it (0x180150020, 0x18015003d). A property of 120 bytes
/// or more comes cut to 119, as the DLL's tracker module keeps it
/// ([`PROPERTY_KEPT`]), so the DLL's own refusal of a string of 128 or more
/// (`TOBII_ERROR_INTERNAL`, 0x18014fff9, 0x18015000e) cannot happen here, as it cannot
/// for a tracker the DLL drives.
///
/// The strings are the ones the tracker reported at its last init, so this
/// answers while it is unplugged, and shares `tobii_get_device_info`'s copy,
/// fetched once per connection; the DLL fixes both when the device is
/// created (its connect copy, which a reconnect delivers again unchanged).
/// `TOBII_ERROR_NOT_SUPPORTED`, as before this library implemented it, from
/// a daemon that does not send them. Nothing is written unless the call
/// succeeds.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `info` must be null or
/// valid for writing one `tobii_device_info_internal_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_get_device_info_internal(
    device: *mut Device,
    info: *mut DeviceInfoInternal,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    if info.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    let internal = match d.device_info() {
        Ok(i) => i.internal,
        Err(status) => return status,
    };
    let Some(internal) = internal else {
        return TOBII_ERROR_NOT_SUPPORTED;
    };
    let mut out = DeviceInfoInternal {
        platform_type: [0; INTERNAL_INFO_STRING],
        property_4: [0; INTERNAL_INFO_STRING],
    };
    copy_c_bytes(
        &mut out.platform_type,
        kept_property(&internal.platform_type),
    );
    copy_c_bytes(&mut out.property_4, kept_property(&internal.property_4));
    // SAFETY: non-null, and the caller guarantees it is writable.
    unsafe { info.write(out) };
    TOBII_ERROR_NO_ERROR
}

/// Face id (`tobii_face_id_*`, PRP properties 0xd `FACE_ID_STATE` and 0xe
/// `FACE_ID_PARAMETERS` and actions 0x1a and 0x1b): every entry point answers
/// `TOBII_ERROR_NOT_SUPPORTED` once its arguments check out, as the DLL does
/// for a tracker it drives itself. Its tracker module lists neither property
/// as readable, writable or subscribable (it never lists its ids 12 and 13,
/// which the server's tables map to them, 0x18016b551..0x18016b8c7), so the
/// DLL's property helpers refuse them before any request, and it hands the
/// enrol actions to stubs that answer not supported (0x18015e4d0,
/// 0x18015e4e0). No licence is checked on the way.
///
/// Enrol a face: the DLL (0x18014a8c0) checks a null device, `face_id` and
/// `result`, then the callback flag, then reads `face_id` (one `u32`) and
/// sends action 0x1b, which its module refuses; `result` is written only on
/// success (0x1800172e8). Nothing is read or written here.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_face_id_enroll(
    device: *mut Device,
    face_id: *const u32,
    result: *mut u32,
) -> Status {
    let missing = face_id.is_null() || result.is_null();
    refused("tobii_face_id_enroll", device, missing)
}

/// Clear an enrolled face: as `tobii_face_id_enroll`, with action 0x1a and
/// no result (0x18014a6a0).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_face_id_enroll_clear(device: *mut Device, face_id: *const u32) -> Status {
    refused("tobii_face_id_enroll_clear", device, face_id.is_null())
}

/// Subscribe to the face id parameters (property 0xe): the DLL (0x18014a230)
/// goes through its `tobii_property_subscribe` (0x18015c860), which checks a
/// null device and callback, then the callback flag, then refuses a property
/// the device's subscribable list lacks (0x18015c9a3).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_face_id_parameters_subscribe(
    device: *mut Device,
    callback: *const c_void,
    _user_data: *mut c_void,
) -> Status {
    let export = "tobii_face_id_parameters_subscribe";
    refused(export, device, callback.is_null())
}

/// The unsubscribe of `tobii_face_id_parameters_subscribe`: never
/// `TOBII_ERROR_NOT_SUBSCRIBED`, as the DLL's `tobii_property_unsubscribe`
/// (0x18015c410) refuses the property (0x18015c4dd) before it looks for a
/// subscription.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_face_id_parameters_unsubscribe(device: *mut Device) -> Status {
    refused("tobii_face_id_parameters_unsubscribe", device, false)
}

/// Subscribe to the face id state (property 0xd): as
/// `tobii_face_id_parameters_subscribe` (0x18014a370).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_face_id_state_subscribe(
    device: *mut Device,
    callback: *const c_void,
    _user_data: *mut c_void,
) -> Status {
    refused("tobii_face_id_state_subscribe", device, callback.is_null())
}

/// The unsubscribe of `tobii_face_id_state_subscribe`: as
/// `tobii_face_id_parameters_unsubscribe` (0x18014a2e0).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_face_id_state_unsubscribe(device: *mut Device) -> Status {
    refused("tobii_face_id_state_unsubscribe", device, false)
}

/// The face id parameters: the DLL (0x18014a540) reads property 0xe through
/// its `tobii_property_get`, which checks a null device and output, then the
/// callback flag, then refuses a property the device's readable list lacks
/// before any request (0x18015d450). Nothing is written.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_get_face_id_parameters(
    device: *mut Device,
    parameters: *mut c_void,
) -> Status {
    let export = "tobii_get_face_id_parameters";
    refused(export, device, parameters.is_null())
}

/// The face id state: as `tobii_get_face_id_parameters`, for property 0xd
/// (0x18014a5f0).
#[unsafe(no_mangle)]
pub extern "C" fn tobii_get_face_id_state(device: *mut Device, state: *mut c_void) -> Status {
    refused("tobii_get_face_id_state", device, state.is_null())
}

/// Set the face id parameters: the DLL (0x18014a420) checks `parameters`
/// before the device, reads them, and writes property 0xe through its
/// `tobii_property_set` (0x18015c150), which checks the callback flag, then
/// refuses a property the device's writable list lacks before any request
/// (0x18015c29a). Nothing is read here.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_set_face_id_parameters(
    device: *mut Device,
    parameters: *const c_void,
) -> Status {
    let export = "tobii_set_face_id_parameters";
    refused(export, device, parameters.is_null())
}

/// The longest string `tobii_send_statistics` takes, its NUL not counted:
/// the DLL's module refuses one of 256 bytes or more (0x18017607d).
const STATISTICS_STRING_MAX: usize = 255;

/// Whether the string at `s` is at most `max` bytes long, reading no more
/// than `max + 1` of them.
///
/// # Safety
/// `s` must point to a NUL-terminated string.
unsafe fn at_most(s: *const c_char, max: usize) -> bool {
    // SAFETY: the caller guarantees a NUL-terminated string, and the search
    // stops at its NUL, so every byte read is the string's.
    (0..=max).any(|i| unsafe { *s.add(i) } == 0)
}

/// Send usage statistics to Tobii: `TOBII_ERROR_NOT_AVAILABLE` once the
/// arguments check out, as the DLL answers for a tracker it drives itself
/// when Tobii's Stream Engine Services do not run, as on Linux. Nothing is
/// sent anywhere.
///
/// The DLL (0x18014b950) takes two strings, `a` and `b` (what they hold is
/// not known), not a buffer and its size: it checks a null device, then each
/// string (0x18014b9f0, 0x18014ba61), then the callback flag (0x18014bb76),
/// and behind Tobii's service answers `TOBII_ERROR_NOT_SUPPORTED`
/// (0x18014bbcf). On its own tracker module (0x180175fd0) a string of 256
/// bytes or more is `TOBII_ERROR_INVALID_PARAMETER` (0x18017607d), and then,
/// as the module never reached the services (`tobii-sesp://`, tried three
/// times at connect, 0x18016b33d), `TOBII_ERROR_NOT_AVAILABLE`
/// (0x180176168); with them it would be `TOBII_ERROR_NOT_SUPPORTED` unless
/// they offer statistics (0x18017632c), and otherwise their answer.
///
/// # Safety
/// `a` and `b` must each be null or point to a NUL-terminated string; at
/// most 256 bytes of each are read. `device` is only compared with null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_send_statistics(
    device: *mut Device,
    a: *const c_char,
    b: *const c_char,
) -> Status {
    if in_callback() {
        return TOBII_ERROR_CALLBACK_IN_PROGRESS;
    }
    if device.is_null() || a.is_null() || b.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    // SAFETY: both are non-null, and the caller guarantees NUL-terminated
    // strings.
    let short = unsafe { at_most(a, STATISTICS_STRING_MAX) && at_most(b, STATISTICS_STRING_MAX) };
    if !short {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    tracing::trace!("tobii_send_statistics: no Stream Engine Services");
    TOBII_ERROR_NOT_AVAILABLE
}

/// Allow or forbid firmware upgrades: `TOBII_ERROR_NOT_SUPPORTED` whatever
/// the arguments, a null device and a call from inside a callback included,
/// as in the DLL (0x18014c4c0), which checks nothing, never reads `allowed`
/// and only logs the refusal. It hands its first argument to its log helper
/// (0x18015e360) as an API instance, which that calls through at +0x20
/// unchecked (0x18015e44d): in the DLL the argument is presumably a
/// `tobii_api_t*` (inferred), and a device handle there would be called
/// through a field of the device. Nothing is read here, so the function is
/// safe.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_set_fw_upgrade_allowed(_device: *mut c_void, _allowed: u32) -> Status {
    tracing::trace!("tobii_set_fw_upgrade_allowed: not supported");
    TOBII_ERROR_NOT_SUPPORTED
}

/// Set the display information: `TOBII_ERROR_NOT_SUPPORTED` once the device
/// and `display_info` check out, as the DLL (0x18014dcb0) answers on every
/// path: it checks a null device (0x18014dcc6), takes its API mutex, checks
/// a null `display_info` (0x18014dd3b) and answers
/// `TOBII_ERROR_NOT_SUPPORTED` (0x18014dd62) without reading it. It has no
/// callback check, so neither has this: from inside a callback the answers
/// are the same. Unlike the DLL's, it waits for no other thread's call. The
/// getter's record (`tobii_get_display_info`, not implemented) is 72 bytes,
/// a 64-byte string and two `int`s (its module function, 0x180175660), which
/// the setter never reads. The pointers are only compared with null, so the
/// function is safe.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_set_display_info(
    device: *mut Device,
    display_info: *const c_void,
) -> Status {
    if device.is_null() || display_info.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    tracing::trace!("tobii_set_display_info: not supported");
    TOBII_ERROR_NOT_SUPPORTED
}

/// The foveated rendering gaze point: the device's gaze point on the
/// display as the DLL hands it out for foveated rendering, which it serves
/// for a tracker it drives itself, from the same gaze frames as raw gaze,
/// whatever the licence. Each raw gaze record gives one, a
/// `tobii_gaze_point_t` held still within 0.02 of the last point that moved
/// it, and timestamped with the frame's host time, which tobiid sends beside
/// raw gaze's tracker time, as the DLL adds its host-minus-tracker offset
/// (see `device::FoveatedPoint`); its callback runs after raw gaze's for
/// the same record. Subscribing asks tobiid for the raw gaze stream, as a
/// raw gaze subscribe does.
///
/// The DLL (0x18014d410) checks a null device, then a null callback, then
/// the callback flag, and under its API mutex refuses a device run by no
/// tracker module of its own (behind Tobii's service) with
/// `TOBII_ERROR_NOT_SUPPORTED` (0x18014d4e8). Its module (0x180176890)
/// checks a licence level below 0, which never is, refuses an occupied slot
/// with `TOBII_ERROR_ALREADY_SUBSCRIBED` (0x180176997), starts the gaze
/// stream the first user of it starts (0x18016e610) and stores the callback
/// (0x180176b1f). Here the subscription follows the stream rules of the
/// other subscribes (see `streams`).
///
/// # Safety
/// As the subscribe functions in `tobii_streams.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_foveated_rendering_gaze_point_subscribe(
    device: *mut Device,
    callback: Option<GazePointFn>,
    user_data: *mut c_void,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { subscribe(device, |c| &mut c.foveated_gaze, callback, user_data) }
}

/// Undo `tobii_foveated_rendering_gaze_point_subscribe`:
/// `TOBII_ERROR_NOT_SUBSCRIBED` when nothing is subscribed, as the DLL's
/// module answers (0x180174afc). The point held stays as it was.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_foveated_rendering_gaze_point_unsubscribe(
    device: *mut Device,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { unsubscribe(device, |c| &mut c.foveated_gaze) }
}

type P = *mut c_void;
type C = *const c_void;

not_supported! {
    fn tobii_clean_ir_subscribe(device: P, callback: C, user_data: P);
    fn tobii_clean_ir_unsubscribe(device: P);
    fn tobii_custom_stream_subscribe(device: P, callback: C, stream: u32, user_data: P);
    fn tobii_custom_stream_unsubscribe(device: P, stream: u32);
    fn tobii_diagnostic_images_retrieve(device: P, receiver: C, user_data: P);
    fn tobii_diagnostics_dump_images(device: P, a: u32, b: u32);
    fn tobii_diagnostics_get_data(device: P, kind: u32, receiver: C, user_data: P);
    fn tobii_diagnostics_image_subscribe(device: P, callback: C, user_data: P);
    fn tobii_diagnostics_image_unsubscribe(device: P);
    fn tobii_display_id_subscribe(device: P, callback: C, user_data: P);
    fn tobii_display_id_unsubscribe(device: P);
    fn tobii_enable_extension(device: P, extension: u32);
    fn tobii_enumerate_enabled_extensions(device: P, receiver: C, user_data: P);
    fn tobii_enumerate_extensions(device: P, receiver: C, user_data: P);
    fn tobii_enumerate_illumination_modes(device: P, receiver: C, user_data: P);
    fn tobii_get_combined_gaze_hid_track_box(device: P, track_box: P);
    fn tobii_get_configuration_key(device: P, key: P, value: P);
    fn tobii_get_display_id(device: P, display_id: P);
    fn tobii_get_display_info(device: P, display_info: P);
    fn tobii_get_gaze_hid_enabled(device: P, enabled: P);
    fn tobii_get_illumination_mode(device: P, mode: P);
    fn tobii_image_collection_subscribe(device: P, callback: C, user_data: P);
    fn tobii_image_collection_unsubscribe(device: P);
    fn tobii_logs_retrieve(device: P, receiver: C, user_data: P);
    fn tobii_open_realm(device: P, realm: u32, key: C, key_size: u32);
    fn tobii_power_save_activate(device: P);
    fn tobii_power_save_deactivate(device: P);
    fn tobii_remote_wake_activate(device: P);
    fn tobii_remote_wake_deactivate(device: P);
    fn tobii_send_custom_command(device: P, command: u32, data: C, size: usize, receiver: C, user_data: P);
    fn tobii_set_display_id(device: P, display_id: u32);
    fn tobii_set_illumination_mode(device: P, mode: C);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tobii_device_destroy;
    use crate::status::{
        TOBII_ERROR_ALREADY_SUBSCRIBED, TOBII_ERROR_CONNECTION_FAILED, TOBII_ERROR_INTERNAL,
        TOBII_ERROR_NOT_AVAILABLE, TOBII_ERROR_NOT_SUBSCRIBED,
    };
    use crate::types::{GazeRaw, TOBII_NOT_SUPPORTED, TOBII_SUPPORTED};
    use std::collections::BTreeSet;
    use std::ffi::CStr;
    use std::ptr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;
    use tobii_ipc::request::{
        COLUMN_STREAM_IDS, Timesync, encode_stream_columns, encode_stream_types, encode_timesync,
        status,
    };

    /// A device whose daemon answers `DEVICE_INFO` with `payload` and counts
    /// the `DEVICE_INFO` requests it gets.
    fn device_info_device(payload: Vec<u8>) -> (*mut Device, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&asked);
        let connect = crate::device::tests::fake_daemon(move |body| match body.first() {
            Some(&tobii_ipc::TAG_SUBSCRIBE) => vec![tobii_ipc::encode_subscribed(true)],
            Some(&tobii_ipc::TAG_REQUEST) => {
                let req = request::decode_request(body).expect("request");
                if req.kind == kind::DEVICE_INFO {
                    seen.fetch_add(1, Ordering::Relaxed);
                    vec![tobii_ipc::encode_reply(req.id, 0, &payload)]
                } else {
                    vec![tobii_ipc::encode_reply(req.id, status::NOT_SUPPORTED, &[])]
                }
            }
            _ => vec![],
        });
        let d = Device::new(connect, 1, 1).expect("device");
        (Box::into_raw(Box::new(d)), asked)
    }

    /// A `DEVICE_INFO` reply carrying `internal`.
    fn device_info_payload(internal: Option<request::DeviceInfoInternal>) -> Vec<u8> {
        request::encode_device_info(&request::DeviceInfo {
            model: "IS5_Large_Eyetracker_5".into(),
            integration_type: "Peripheral".into(),
            internal,
            ..request::DeviceInfo::default()
        })
    }

    fn internal_info(platform_type: &str, property_4: &str) -> Option<request::DeviceInfoInternal> {
        Some(request::DeviceInfoInternal {
            platform_type: platform_type.into(),
            property_4: property_4.into(),
        })
    }

    /// A `tobii_device_info_internal_t` the call has not touched.
    fn untouched_internal() -> DeviceInfoInternal {
        DeviceInfoInternal {
            platform_type: [0x5a; INTERNAL_INFO_STRING],
            property_4: [0x5a; INTERNAL_INFO_STRING],
        }
    }

    /// A C string field's bytes up to its NUL, and whether every byte after
    /// it is zero.
    fn c_field(field: &[c_char]) -> (Vec<u8>, bool) {
        let bytes: Vec<u8> = field.iter().map(|c| c.to_ne_bytes()[0]).collect();
        let n = bytes.iter().position(|b| *b == 0).expect("NUL-terminated");
        (bytes[..n].to_vec(), bytes[n..].iter().all(|b| *b == 0))
    }

    #[test]
    fn the_internal_device_info_is_properties_3_and_4_zero_padded() {
        let (d, asked) =
            device_info_device(device_info_payload(internal_info("IS5LEYETRACKER5", "")));
        let mut info = untouched_internal();
        let mut public = crate::api::device_info_c(&request::DeviceInfo::default());
        // SAFETY: `d` is live and destroyed once; `info` and `public` are
        // live locals.
        unsafe {
            assert_eq!(tobii_get_device_info_internal(d, &raw mut info), 0);
            assert_eq!(
                c_field(&info.platform_type),
                (b"IS5LEYETRACKER5".to_vec(), true)
            );
            assert_eq!(
                c_field(&info.property_4),
                (Vec::new(), true),
                "no property 4"
            );
            assert_eq!(crate::api::tobii_get_device_info(d, &raw mut public), 0);
            assert_eq!(tobii_device_destroy(d), 0);
        }
        assert_eq!(
            asked.load(Ordering::Relaxed),
            1,
            "one fetch per connection, shared with tobii_get_device_info"
        );
    }

    #[test]
    fn a_daemon_without_the_internal_properties_answers_not_supported() {
        let five = device_info_payload(None);
        let mut four = five.clone();
        // Drop the integration type: what a daemon from before it sends.
        four.truncate(four.len() - "Peripheral".len() - 2);
        for payload in [five, four] {
            let (d, _) = device_info_device(payload);
            let mut info = untouched_internal();
            // SAFETY: `d` is live and destroyed once; `info` a live local.
            unsafe {
                assert_eq!(
                    tobii_get_device_info_internal(d, &raw mut info),
                    TOBII_ERROR_NOT_SUPPORTED,
                    "as before libtobii implemented it"
                );
                assert_eq!(tobii_device_destroy(d), 0);
            }
            assert_eq!(info, untouched_internal(), "nothing written");
        }
    }

    /// The DLL's tracker module keeps 119 bytes of a property, so its own
    /// refusal of 128 or more never fires: neither does libtobii's.
    #[test]
    fn a_property_is_cut_to_119_bytes_as_the_dlls_tracker_module_keeps_it() {
        let kept = "x".repeat(PROPERTY_KEPT);
        let longer = format!("{kept}yz");
        let too_long = "y".repeat(INTERNAL_INFO_STRING * 3);
        for (p3, p4, expected_3, expected_4) in [
            (kept.as_str(), "", kept.as_bytes(), &b""[..]),
            (longer.as_str(), "", kept.as_bytes(), b""),
            (
                "IS5LEYETRACKER5",
                too_long.as_str(),
                b"IS5LEYETRACKER5",
                &too_long.as_bytes()[..119],
            ),
        ] {
            let (d, _) = device_info_device(device_info_payload(internal_info(p3, p4)));
            let mut info = untouched_internal();
            // SAFETY: `d` is live and destroyed once; `info` a live local.
            unsafe {
                assert_eq!(tobii_get_device_info_internal(d, &raw mut info), 0);
                assert_eq!(tobii_device_destroy(d), 0);
            }
            assert_eq!(c_field(&info.platform_type), (expected_3.to_vec(), true));
            assert_eq!(c_field(&info.property_4), (expected_4.to_vec(), true));
        }
    }

    /// The DLL measures and copies with C string functions, so a string is
    /// what comes before its first NUL, and zero follows it.
    #[test]
    fn a_property_ends_at_its_first_nul() {
        let tail = "y".repeat(INTERNAL_INFO_STRING * 2);
        let p3 = format!("IS5\0{tail}");
        let (d, _) = device_info_device(device_info_payload(internal_info(&p3, "")));
        let mut info = untouched_internal();
        // SAFETY: `d` is live and destroyed once; `info` a live local.
        unsafe {
            assert_eq!(tobii_get_device_info_internal(d, &raw mut info), 0);
            assert_eq!(tobii_device_destroy(d), 0);
        }
        assert_eq!(c_field(&info.platform_type), (b"IS5".to_vec(), true));
    }

    #[test]
    fn the_internal_device_info_checks_its_arguments_first() {
        let (d, frames) = crate::device::tests::counting_device();
        let mut info = untouched_internal();
        // SAFETY: `d` is a live handle, destroyed by `destroy_untouched`;
        // `info` a live local.
        unsafe {
            assert_eq!(
                tobii_get_device_info_internal(ptr::null_mut(), &raw mut info),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_get_device_info_internal(d, ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            crate::device::call(|| {
                assert_eq!(
                    tobii_get_device_info_internal(d, &raw mut info),
                    TOBII_ERROR_CALLBACK_IN_PROGRESS
                );
            });
        }
        assert_eq!(info, untouched_internal(), "nothing written");
        crate::device::tests::destroy_untouched(d, &frames);
    }

    /// A fetch that fails answers its own error, as for
    /// `tobii_get_device_info`, and the output stays as it was: the DLL
    /// writes nothing before its checks pass (0x180150017).
    #[test]
    fn a_failed_device_info_fetch_answers_its_error_and_writes_nothing() {
        for (reply, payload, expected) in [
            // tobiid has no tracker to ask, or lost it.
            (
                status::CONNECTION_FAILED,
                vec![],
                TOBII_ERROR_CONNECTION_FAILED,
            ),
            // A reply that does not decode.
            (status::OK, vec![0xff], TOBII_ERROR_INTERNAL),
        ] {
            let d = Box::into_raw(Box::new(crate::device::tests::device_with(reply, payload)));
            let mut info = untouched_internal();
            // SAFETY: `d` is live and destroyed once; `info` a live local.
            let got = unsafe {
                let got = tobii_get_device_info_internal(d, &raw mut info);
                assert_eq!(tobii_device_destroy(d), 0);
                got
            };
            assert_eq!(got, expected, "reply status {reply}");
            assert_eq!(info, untouched_internal(), "nothing written");
        }
    }

    #[test]
    fn only_the_ir_image_is_a_supported_internal_stream() {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(0, vec![])));
        let mut s = 9u32;
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `s` is a live local.
        unsafe {
            assert_eq!(tobii_internal_stream_supported(d, 0, &raw mut s), 0);
            assert_eq!(s, TOBII_SUPPORTED);
            // For 3, 4, 5, 7 and 8 this is the DLL's own answer for a tracker
            // it drives itself.
            for stream in (1..=8).chain([9, 1000, 0x7fff_ffff]) {
                s = 9;
                assert_eq!(
                    tobii_internal_stream_supported(d, stream, &raw mut s),
                    0,
                    "{stream}: unknown is not an error"
                );
                assert_eq!(s, TOBII_NOT_SUPPORTED, "{stream}");
            }
            s = 9;
            assert_eq!(
                tobii_internal_stream_supported(d, 0x8000_0000, &raw mut s),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(s, 9, "nothing written");
            assert_eq!(
                tobii_internal_stream_supported(d, 0, ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_internal_stream_supported(ptr::null_mut(), 0, &raw mut s),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(s, 9, "nothing written");
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    type RefusedSubscribe = extern "C" fn(*mut Device, *const c_void, *mut c_void) -> Status;
    type RefusedUnsubscribe = extern "C" fn(*mut Device) -> Status;

    /// The subscribe and unsubscribe of each internal stream the DLL refuses
    /// for a tracker it drives itself.
    const REFUSED: [(u32, RefusedSubscribe, RefusedUnsubscribe); 5] = [
        (
            3,
            tobii_low_frequency_head_rotation_subscribe,
            tobii_low_frequency_head_rotation_unsubscribe,
        ),
        (
            4,
            tobii_low_frequency_head_position_subscribe,
            tobii_low_frequency_head_position_unsubscribe,
        ),
        (
            5,
            tobii_multiple_faces_position_subscribe,
            tobii_multiple_faces_position_unsubscribe,
        ),
        (
            7,
            tobii_wearable_limited_image_subscribe,
            tobii_wearable_limited_image_unsubscribe,
        ),
        (
            8,
            tobii_secondary_camera_image_subscribe,
            tobii_secondary_camera_image_unsubscribe,
        ),
    ];

    /// A callback the refused subscribes are given and never call.
    fn never_called() -> *const c_void {
        ptr::NonNull::<c_void>::dangling().as_ptr().cast_const()
    }

    unsafe extern "C" fn ignore_head_pose(_: *const crate::types::HeadPose, _: *mut c_void) {}

    /// The daemon fails every request, as with no tracker plugged in, and
    /// counts every frame: only the head-pose subscribe at the end reaches
    /// it. An unsubscribe is refused, not `TOBII_ERROR_NOT_SUBSCRIBED`, as in
    /// the DLL, whose support check comes first.
    #[test]
    fn refused_internal_streams_are_not_supported_without_asking_the_daemon() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tobii_ipc::request::decode_request;
        use tobii_ipc::{encode_reply, encode_subscribed};
        let frames = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&frames);
        let connect = crate::device::tests::fake_daemon(move |body| {
            seen.fetch_add(1, Ordering::Relaxed);
            match body.first() {
                Some(&tobii_ipc::TAG_SUBSCRIBE) => vec![encode_subscribed(true)],
                Some(&tobii_ipc::TAG_REQUEST) => {
                    let req = decode_request(body).expect("request");
                    vec![encode_reply(req.id, status::CONNECTION_FAILED, &[])]
                }
                _ => vec![],
            }
        });
        let d = Box::into_raw(Box::new(Device::new(connect, 1, 1).expect("device")));
        let (refused, invalid) = (TOBII_ERROR_NOT_SUPPORTED, TOBII_ERROR_INVALID_PARAMETER);
        let n = ptr::null_mut();
        for (stream, subscribe, unsubscribe) in REFUSED {
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
            assert_eq!(got, want, "internal stream {stream}");
        }
        assert_eq!(frames.load(Ordering::Relaxed), 0, "nothing sent");
        // The daemon handles frames in order, so once the head-pose subscribe
        // is acked, a frame the calls sent without waiting would have been
        // counted too. The refused subscribes left head pose's slot free.
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once.
        unsafe {
            assert_eq!(
                crate::streams::tobii_head_pose_subscribe(d, Some(ignore_head_pose), n),
                TOBII_ERROR_NO_ERROR
            );
            assert_eq!(frames.load(Ordering::Relaxed), 1, "only the head pose");
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    /// Inside a callback every call is refused before its arguments are
    /// read, as wherever a device handle is taken. The DLL checks the nulls
    /// first, so for those it would answer `TOBII_ERROR_INVALID_PARAMETER`.
    #[test]
    fn refused_internal_streams_are_refused_inside_a_callback() {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(0, vec![])));
        let n = ptr::null_mut();
        let mut got = Vec::new();
        crate::device::call(|| {
            for (_, subscribe, unsubscribe) in REFUSED {
                got.extend([
                    subscribe(d, never_called(), n),
                    subscribe(n.cast(), never_called(), n),
                    subscribe(d, ptr::null(), n),
                    subscribe(n.cast(), ptr::null(), n),
                    unsubscribe(d),
                    unsubscribe(n.cast()),
                ]);
            }
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 30]);
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once;
        // the callback guard is down again.
        unsafe { assert_eq!(tobii_device_destroy(d), 0) };
    }

    /// The face id subscribe and unsubscribe pairs.
    const FACE_ID_STREAMS: [(&str, RefusedSubscribe, RefusedUnsubscribe); 2] = [
        (
            "state",
            tobii_face_id_state_subscribe,
            tobii_face_id_state_unsubscribe,
        ),
        (
            "parameters",
            tobii_face_id_parameters_subscribe,
            tobii_face_id_parameters_unsubscribe,
        ),
    ];

    /// A face id state or parameters record (at most 8 bytes) no call may
    /// write.
    const FACE_ID_RECORD: [u8; 8] = [0xa5; 8];

    /// The daemon counts every frame: none reaches it. The unsubscribes are
    /// refused, never `TOBII_ERROR_NOT_SUBSCRIBED`, as in the DLL.
    #[test]
    fn face_id_is_not_supported_without_asking_the_daemon() {
        let (d, frames) = crate::device::tests::counting_device();
        let (refused, invalid) = (TOBII_ERROR_NOT_SUPPORTED, TOBII_ERROR_INVALID_PARAMETER);
        let n: *mut c_void = ptr::null_mut();
        let face_id = 7_u32;
        let mut result = 0xdead_beef_u32;
        let mut record = FACE_ID_RECORD;
        let out = record.as_mut_ptr().cast::<c_void>();
        let got = [
            tobii_face_id_enroll(d, &face_id, &raw mut result),
            tobii_face_id_enroll(d, ptr::null(), &raw mut result),
            tobii_face_id_enroll(d, &face_id, ptr::null_mut()),
            tobii_face_id_enroll(n.cast(), &face_id, &raw mut result),
            tobii_face_id_enroll_clear(d, &face_id),
            tobii_face_id_enroll_clear(d, ptr::null()),
            tobii_face_id_enroll_clear(n.cast(), &face_id),
            tobii_get_face_id_state(d, out),
            tobii_get_face_id_state(d, n),
            tobii_get_face_id_state(n.cast(), out),
            tobii_get_face_id_parameters(d, out),
            tobii_get_face_id_parameters(d, n),
            tobii_get_face_id_parameters(n.cast(), out),
            tobii_set_face_id_parameters(d, out.cast_const()),
            tobii_set_face_id_parameters(d, ptr::null()),
            tobii_set_face_id_parameters(n.cast(), out.cast_const()),
        ];
        let want = [
            refused, invalid, invalid, invalid, refused, invalid, invalid, refused, invalid,
            invalid, refused, invalid, invalid, refused, invalid, invalid,
        ];
        assert_eq!(got, want);
        for (property, subscribe, unsubscribe) in FACE_ID_STREAMS {
            let got = [
                unsubscribe(d),
                subscribe(d, never_called(), n),
                subscribe(d, ptr::null(), n),
                subscribe(n.cast(), never_called(), n),
                unsubscribe(d),
                unsubscribe(n.cast()),
            ];
            let want = [refused, refused, invalid, invalid, refused, invalid];
            assert_eq!(got, want, "face id {property}");
        }
        assert_eq!(record, FACE_ID_RECORD, "nothing written");
        assert_eq!(result, 0xdead_beef, "no result written");
        crate::device::tests::destroy_untouched(d, &frames);
    }

    /// Inside a callback every face id call is refused before its arguments
    /// are read; the DLL checks the nulls first.
    #[test]
    fn face_id_is_refused_inside_a_callback() {
        let (d, frames) = crate::device::tests::counting_device();
        let n: *mut c_void = ptr::null_mut();
        let face_id = 7_u32;
        let mut result = 0xdead_beef_u32;
        let mut record = FACE_ID_RECORD;
        let out = record.as_mut_ptr().cast::<c_void>();
        let mut got = Vec::new();
        crate::device::call(|| {
            got.extend([
                tobii_face_id_enroll(d, &face_id, &raw mut result),
                tobii_face_id_enroll(n.cast(), ptr::null(), ptr::null_mut()),
                tobii_face_id_enroll_clear(d, &face_id),
                tobii_face_id_enroll_clear(n.cast(), ptr::null()),
                tobii_get_face_id_state(d, out),
                tobii_get_face_id_state(n.cast(), n),
                tobii_get_face_id_parameters(d, out),
                tobii_get_face_id_parameters(n.cast(), n),
                tobii_set_face_id_parameters(d, out.cast_const()),
                tobii_set_face_id_parameters(n.cast(), ptr::null()),
            ]);
            for (_, subscribe, unsubscribe) in FACE_ID_STREAMS {
                got.extend([
                    subscribe(d, never_called(), n),
                    subscribe(n.cast(), ptr::null(), n),
                    unsubscribe(d),
                    unsubscribe(n.cast()),
                ]);
            }
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 18]);
        assert_eq!(record, FACE_ID_RECORD, "nothing written");
        assert_eq!(result, 0xdead_beef, "no result written");
        crate::device::tests::destroy_untouched(d, &frames);
    }

    /// Statistics are two strings of at most 255 bytes: past those checks
    /// the answer is `TOBII_ERROR_NOT_AVAILABLE`, as the DLL's for a tracker
    /// it drives itself without Tobii's Stream Engine Services. The display
    /// information is refused once its pointers check out, and the firmware
    /// upgrade switch whatever it is given. The daemon counts no frame.
    #[test]
    fn statistics_display_info_and_firmware_answer_without_asking_the_daemon() {
        let (d, frames) = crate::device::tests::counting_device();
        let n: *mut c_void = ptr::null_mut();
        let (invalid, refused) = (TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NOT_SUPPORTED);
        let longest = [b'a'; 255]
            .iter()
            .chain(b"\0")
            .copied()
            .collect::<Vec<u8>>();
        let too_long = [b'a'; 256]
            .iter()
            .chain(b"\0")
            .copied()
            .collect::<Vec<u8>>();
        let (s, l, t) = (
            c"key".as_ptr(),
            longest.as_ptr().cast(),
            too_long.as_ptr().cast(),
        );
        // SAFETY: every non-null string is NUL-terminated and outlives the
        // call; the device is only compared with null.
        let statistics = unsafe {
            [
                tobii_send_statistics(d, s, s),
                tobii_send_statistics(d, l, l),
                tobii_send_statistics(d, t, s),
                tobii_send_statistics(d, s, t),
                tobii_send_statistics(d, ptr::null(), s),
                tobii_send_statistics(d, s, ptr::null()),
                tobii_send_statistics(n.cast(), s, s),
            ]
        };
        let unavailable = TOBII_ERROR_NOT_AVAILABLE;
        assert_eq!(
            statistics,
            [
                unavailable,
                unavailable,
                invalid,
                invalid,
                invalid,
                invalid,
                invalid
            ]
        );
        let mut info = [0x5a_u8; 72];
        let i = info.as_mut_ptr().cast::<c_void>().cast_const();
        let got = [
            tobii_set_display_info(d, i),
            tobii_set_display_info(d, ptr::null()),
            tobii_set_display_info(n.cast(), i),
            tobii_set_fw_upgrade_allowed(d.cast(), 1),
            tobii_set_fw_upgrade_allowed(n, 0),
        ];
        assert_eq!(got, [refused, invalid, invalid, refused, refused]);
        assert_eq!(info, [0x5a; 72], "nothing written");
        crate::device::tests::destroy_untouched(d, &frames);
    }

    /// Inside a callback statistics are refused before their arguments are
    /// read (the DLL checks the nulls first), while the display information
    /// and the firmware upgrade switch answer as outside one: the DLL never
    /// checks for a callback in them.
    #[test]
    fn only_statistics_are_refused_inside_a_callback() {
        let (d, frames) = crate::device::tests::counting_device();
        let n: *mut c_void = ptr::null_mut();
        let s = c"key".as_ptr();
        let info = [0x5a_u8; 72];
        let i = info.as_ptr().cast::<c_void>();
        let mut got = Vec::new();
        crate::device::call(|| {
            // SAFETY: as in the test above.
            got.extend(unsafe {
                [
                    tobii_send_statistics(d, s, s),
                    tobii_send_statistics(n.cast(), ptr::null(), ptr::null()),
                ]
            });
            got.extend([
                tobii_set_display_info(d, i),
                tobii_set_display_info(n.cast(), i),
                tobii_set_fw_upgrade_allowed(d.cast(), 1),
                tobii_set_fw_upgrade_allowed(n, 1),
            ]);
        });
        let (in_callback, invalid) = (
            TOBII_ERROR_CALLBACK_IN_PROGRESS,
            TOBII_ERROR_INVALID_PARAMETER,
        );
        let refused = TOBII_ERROR_NOT_SUPPORTED;
        assert_eq!(
            got,
            [in_callback, in_callback, refused, invalid, refused, refused]
        );
        crate::device::tests::destroy_untouched(d, &frames);
    }

    /// What a test's foveated and raw gaze callbacks saw, in order.
    #[derive(Default)]
    struct FoveatedSeen {
        /// `('f', stamp)` or `('r', stamp)`, in the order the calls came.
        order: Vec<(char, i64)>,
        /// The foveated points, as `device::tests::point_bits`.
        points: Vec<(i64, u32, [u32; 2])>,
    }

    unsafe extern "C" fn see_foveated(p: *const crate::types::GazePoint, ud: *mut c_void) {
        // SAFETY: the test passes `&raw mut` of a live `FoveatedSeen` as
        // `ud`, and the library a valid point.
        let (seen, p) = unsafe { (&mut *ud.cast::<FoveatedSeen>(), &*p) };
        seen.order.push(('f', p.timestamp_us));
        seen.points.push(crate::device::tests::point_bits(p));
    }

    unsafe extern "C" fn see_raw(p: *const GazeRaw, ud: *mut c_void) {
        // SAFETY: as `see_foveated`.
        let (seen, p) = unsafe { (&mut *ud.cast::<FoveatedSeen>(), &*p) };
        seen.order.push(('r', p.timestamp_tracker_us));
    }

    /// What `raw_frame` adds to a frame's tracker time for its host time.
    const HOST: i64 = 1_000_000;

    /// A raw gaze frame stamped `ts`, and `ts + HOST` on the host clock if
    /// `timed` (a tobiid from before the host time sends none), with both
    /// eyes at `xy`, or lost.
    fn raw_frame(ts: i64, timed: bool, xy: Option<[f32; 2]>) -> Vec<u8> {
        let eye = tobii_ipc::GazeRawEye {
            status: if xy.is_some() { 0 } else { 4 },
            gaze_point_on_display: xy.unwrap_or([0.9; 2]),
            ..tobii_ipc::GazeRawEye::default()
        };
        tobii_ipc::encode_gaze_raw(&tobii_ipc::GazeRaw {
            timestamp_tracker_us: ts,
            left: eye,
            right: eye,
            timestamp_system_us: timed.then_some(ts + HOST),
            ..tobii_ipc::GazeRaw::default()
        })
    }

    /// The foveated gaze point is fed from raw gaze, whose stream bit its
    /// subscribe asks for, after raw gaze's callback for the same record,
    /// stamped with the frame's host time where raw gaze keeps the tracker's
    /// (the tracker's again from a tobiid that sends no host time); it
    /// follows the stream rules; and its held point survives an unsubscribe
    /// and a reconnect, as the DLL's module keeps it, while samples that come
    /// unsubscribed do not move it.
    #[test]
    fn the_foveated_gaze_point_comes_from_raw_gaze_and_keeps_its_point() {
        use std::sync::{Arc, Mutex};
        use tobii_ipc::{STREAM_GAZE_RAW, decode_subscribe, encode_subscribed};
        let masks = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&masks);
        let connect = crate::device::tests::fake_daemon(move |body| match body.first() {
            Some(&tobii_ipc::TAG_SUBSCRIBE) => {
                log.lock().expect("log").push(decode_subscribe(body));
                vec![encode_subscribed(true)]
            }
            _ => vec![],
        });
        let device = Device::new(connect, 1, 1).expect("device");
        let d = Box::into_raw(Box::new(device));
        let mut seen = FoveatedSeen::default();
        let ud = (&raw mut seen).cast::<c_void>();
        let n = ptr::null_mut();
        let callback = Some(see_foveated as crate::types::GazePointFn);
        // Deliver `frames` as the daemon would have sent them.
        let deliver = |frames: Vec<Vec<u8>>| {
            for f in frames {
                let msg = tobii_ipc::decode_server(&f).expect("decodes");
                // SAFETY: `d` is live until destroyed below.
                unsafe { &*d }.queue(msg);
            }
            // SAFETY: as above.
            assert_eq!(unsafe { crate::api::tobii_device_process_callbacks(d) }, 0);
        };
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `seen` outlives every call that may call back with it.
        unsafe {
            let got = [
                tobii_foveated_rendering_gaze_point_subscribe(d, None, n),
                tobii_foveated_rendering_gaze_point_subscribe(n.cast(), callback, ud),
                tobii_foveated_rendering_gaze_point_unsubscribe(d),
                tobii_foveated_rendering_gaze_point_subscribe(d, callback, ud),
                tobii_foveated_rendering_gaze_point_subscribe(d, callback, ud),
                tobii_gaze_raw_subscribe(d, Some(see_raw as GazeRawFn), ud),
            ];
            let want = [
                TOBII_ERROR_INVALID_PARAMETER,
                TOBII_ERROR_INVALID_PARAMETER,
                TOBII_ERROR_NOT_SUBSCRIBED,
                TOBII_ERROR_NO_ERROR,
                TOBII_ERROR_ALREADY_SUBSCRIBED,
                TOBII_ERROR_NO_ERROR,
            ];
            assert_eq!(got, want);
            deliver(vec![
                raw_frame(1, true, None),
                raw_frame(2, true, Some([0.25, 0.5])),
            ]);
            assert_eq!(tobii_gaze_raw_unsubscribe(d), TOBII_ERROR_NO_ERROR);
            assert_eq!(tobii_foveated_rendering_gaze_point_unsubscribe(d), 0);
            assert_eq!(tobii_foveated_rendering_gaze_point_unsubscribe(d), 12);
            deliver(vec![raw_frame(3, true, Some([0.75, 0.75]))]);
            assert_eq!(
                tobii_foveated_rendering_gaze_point_subscribe(d, callback, ud),
                0
            );
            assert_eq!(crate::api::tobii_device_reconnect(d), TOBII_ERROR_NO_ERROR);
            deliver(vec![raw_frame(4, true, None), raw_frame(5, false, None)]);
            assert_eq!(tobii_foveated_rendering_gaze_point_unsubscribe(n.cast()), 8);
            assert_eq!(tobii_device_destroy(d), 0);
        }
        let (valid, invalid) = (
            crate::types::TOBII_VALIDITY_VALID,
            crate::types::TOBII_VALIDITY_INVALID,
        );
        let max = f32::MAX.to_bits();
        let held = [0.25_f32.to_bits(), 0.5_f32.to_bits()];
        assert_eq!(
            seen.points,
            [
                (1 + HOST, invalid, [max, max]),
                (2 + HOST, valid, held),
                (4 + HOST, invalid, held),
                (5, invalid, held),
            ]
        );
        assert_eq!(
            seen.order,
            [
                ('r', 1),
                ('f', 1 + HOST),
                ('r', 2),
                ('f', 2 + HOST),
                ('f', 4 + HOST),
                ('f', 5),
            ]
        );
        let raw = Some(STREAM_GAZE_RAW);
        assert_eq!(
            *masks.lock().expect("log"),
            [raw, Some(0), raw, raw],
            "foveated, then both, gone, foveated again, after the reconnect"
        );
    }

    /// Inside a callback both calls are refused, as wherever a device handle
    /// is taken.
    #[test]
    fn the_foveated_gaze_point_is_refused_inside_a_callback() {
        let (d, frames) = crate::device::tests::counting_device();
        let n: *mut c_void = ptr::null_mut();
        let callback = Some(see_foveated as crate::types::GazePointFn);
        let mut got = Vec::new();
        crate::device::call(|| {
            // SAFETY: `d` is a live handle; the callback is never called.
            got.extend(unsafe {
                [
                    tobii_foveated_rendering_gaze_point_subscribe(d, callback, n),
                    tobii_foveated_rendering_gaze_point_subscribe(n.cast(), None, n),
                    tobii_foveated_rendering_gaze_point_unsubscribe(d),
                    tobii_foveated_rendering_gaze_point_unsubscribe(n.cast()),
                ]
            });
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 4]);
        crate::device::tests::destroy_untouched(d, &frames);
    }

    unsafe extern "C" fn ignore_raw(_: *const GazeRaw, _: *mut c_void) {}

    /// Raw gaze is subscribed with a bit of its own, and follows the Stream
    /// Engine's subscription rules. The daemon stand-in acks every change
    /// and never sends the stream, as a daemon older than this library
    /// would: the subscribe succeeds all the same.
    #[test]
    fn raw_gaze_takes_its_own_stream_bit_by_the_stream_engines_rules() {
        use std::sync::{Arc, Mutex};
        use tobii_ipc::{STREAM_GAZE_RAW, decode_subscribe, encode_subscribed};
        let masks = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&masks);
        let connect = crate::device::tests::fake_daemon(move |body| match body.first() {
            Some(&tobii_ipc::TAG_SUBSCRIBE) => {
                log.lock().expect("log").push(decode_subscribe(body));
                vec![encode_subscribed(true)]
            }
            _ => vec![],
        });
        let d = Box::into_raw(Box::new(Device::new(connect, 1, 1).expect("device")));
        let n = ptr::null_mut();
        let callback = Some(ignore_raw as GazeRawFn);
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; a null device is allowed; the callback ignores its
        // arguments.
        let got = unsafe {
            let got = [
                tobii_gaze_raw_subscribe(d, None, n),
                tobii_gaze_raw_subscribe(n.cast(), callback, n),
                tobii_gaze_raw_unsubscribe(d),
                tobii_gaze_raw_subscribe(d, callback, n),
                tobii_gaze_raw_subscribe(d, callback, n),
                tobii_gaze_raw_unsubscribe(d),
                tobii_gaze_raw_unsubscribe(d),
                tobii_gaze_raw_unsubscribe(n.cast()),
            ];
            assert_eq!(tobii_device_destroy(d), 0);
            got
        };
        let want = [
            TOBII_ERROR_INVALID_PARAMETER,
            TOBII_ERROR_INVALID_PARAMETER,
            TOBII_ERROR_NOT_SUBSCRIBED,
            TOBII_ERROR_NO_ERROR,
            TOBII_ERROR_ALREADY_SUBSCRIBED,
            TOBII_ERROR_NO_ERROR,
            TOBII_ERROR_NOT_SUBSCRIBED,
            TOBII_ERROR_INVALID_PARAMETER,
        ];
        assert_eq!(got, want);
        assert_eq!(
            *masks.lock().expect("log"),
            [Some(STREAM_GAZE_RAW), Some(0)]
        );
    }

    /// What the raw gaze callback saw.
    #[derive(Default)]
    struct SeenRaw {
        records: Vec<GazeRaw>,
        /// The record's bytes as the callback read them.
        bytes: Vec<u8>,
        /// Whether the callback guard was up while it ran.
        guarded: bool,
    }

    unsafe extern "C" fn keep_raw(p: *const GazeRaw, ud: *mut c_void) {
        // SAFETY: the test passes `&raw mut SeenRaw` as `ud`, which outlives
        // the device; `p` is a live record for the call.
        let (seen, record) = unsafe { (&mut *ud.cast::<SeenRaw>(), &*p) };
        seen.records.push(*record);
        // SAFETY: `record` is live for the call, and all its bytes are
        // initialised: `GazeRaw` has no padding (see the layout test).
        let bytes = unsafe {
            std::slice::from_raw_parts(ptr::from_ref(record).cast::<u8>(), size_of::<GazeRaw>())
        };
        seen.bytes = bytes.to_vec();
        seen.guarded = in_callback();
    }

    /// A raw gaze frame reaches the callback in the DLL's layout: every value
    /// as the daemon sent it, a lost eye's included, the tracker time
    /// unchanged (the host time sent beside it unused), a flagged key's
    /// validity 1 when the frame had it (a 0 sent
    /// too) and 0, with a value of 0, when it did not, and the slots the
    /// DLL's record never fills zero. The callback runs under the callback
    /// guard.
    #[test]
    fn raw_gaze_reaches_its_callback_in_the_dlls_layout() {
        use crate::types::{TOBII_VALIDITY_INVALID as NO, TOBII_VALIDITY_VALID as YES};
        use tobii_ipc::{STREAM_GAZE_RAW, decode_subscribe, encode_gaze_raw, encode_subscribed};
        let sample = tobii_ipc::GazeRaw {
            timestamp_tracker_us: 9_613_320_391,
            left: tobii_ipc::GazeRawEye {
                gaze_origin_mm: [-31.5, 12.25, 612.0],
                gaze_origin_in_track_box: [0.25, 0.5, 0.75],
                gaze_point_mm: [40.5, 292.5, 99.625],
                gaze_point_on_display: [0.5625, 0.125],
                pupil_diameter_mm: 6.25,
                status: 0,
            },
            // Lost, its values passed on all the same.
            right: tobii_ipc::GazeRawEye {
                gaze_origin_mm: [31.5, 11.0, 610.5],
                gaze_origin_in_track_box: [0.375, 0.625, 0.875],
                gaze_point_mm: [-2.0, 1.5, -0.5],
                gaze_point_on_display: [-1.0, 2.0],
                pupil_diameter_mm: 5.75,
                status: 4,
            },
            combined_gaze_point_on_display: [0.5, 0.25],
            combined_gaze_validity: 1,
            key_0e: None,
            key_11: Some(4),
            frame_counter: Some(43_780),
            left_origin_flag: Some(1),
            right_origin_flag: Some(0),
            left_eyeball_center_mm: Some([-30.0, 10.5, 620.25]),
            right_eyeball_center_mm: None,
            timestamp_system_us: Some(1_208_455_092_731),
        };
        let frame = encode_gaze_raw(&sample);
        // Sent ahead of the ack of raw gaze alone, so that it waits for
        // `process`.
        let connect = crate::device::tests::fake_daemon(move |body| match body.first() {
            Some(&tobii_ipc::TAG_SUBSCRIBE) if decode_subscribe(body) == Some(STREAM_GAZE_RAW) => {
                vec![frame.clone(), encode_subscribed(true)]
            }
            Some(&tobii_ipc::TAG_SUBSCRIBE) => vec![encode_subscribed(true)],
            _ => vec![],
        });
        let d = Box::into_raw(Box::new(Device::new(connect, 1, 1).expect("device")));
        let mut seen = SeenRaw::default();
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `keep_raw` gets the live `seen`, which outlives the device.
        unsafe {
            assert_eq!(
                tobii_gaze_raw_subscribe(
                    d,
                    Some(keep_raw as GazeRawFn),
                    (&raw mut seen).cast::<c_void>()
                ),
                TOBII_ERROR_NO_ERROR
            );
            assert_eq!(
                crate::api::tobii_device_process_callbacks(d),
                TOBII_ERROR_NO_ERROR
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }

        let eye = |e: &tobii_ipc::GazeRawEye| crate::types::GazeRawEye {
            gaze_origin_from_eye_tracker_mm_xyz: e.gaze_origin_mm,
            gaze_origin_in_track_box_normalized_xyz: e.gaze_origin_in_track_box,
            gaze_point_from_eye_tracker_mm_xyz: e.gaze_point_mm,
            gaze_point_on_display_normalized_xy: e.gaze_point_on_display,
            pupil_diameter_mm: e.pupil_diameter_mm,
            status: e.status,
        };
        let want = GazeRaw {
            timestamp_tracker_us: 9_613_320_391,
            left: eye(&sample.left),
            right: eye(&sample.right),
            combined_gaze_point_on_display_normalized_xy: [0.5, 0.25],
            combined_gaze_validity: 1,
            key_0e_validity: NO,
            key_0e: 0,
            reserved_84: NO,
            reserved_88: 0.0,
            reserved_8c: NO,
            reserved_90: 0.0,
            key_11_validity: YES,
            key_11: 4,
            reserved_9c: NO,
            reserved_a0: 0.0,
            reserved_a4: NO,
            reserved_a8: 0.0,
            frame_counter_validity: YES,
            frame_counter: 43_780,
            left_origin_flag_validity: YES,
            left_origin_flag: 1,
            right_origin_flag_validity: YES,
            right_origin_flag: 0,
            left_eyeball_center_validity: YES,
            left_eyeball_center_from_eye_tracker_mm_xyz: [-30.0, 10.5, 620.25],
            right_eyeball_center_validity: NO,
            right_eyeball_center_from_eye_tracker_mm_xyz: [0.0; 3],
            reserved_e4: 0,
        };
        assert_eq!(seen.records, [want]);
        assert!(seen.guarded, "run under the callback guard");
        // The bytes the DLL's dispatch leaves zero, and those of the keys
        // the frame lacked: 0x0e's pair and the right eyeball.
        let zero = |range: std::ops::Range<usize>| seen.bytes[range].iter().all(|&b| b == 0);
        assert_eq!(seen.bytes.len(), 232);
        for range in [0x7c..0x84, 0x84..0x94, 0x9c..0xac, 0xd4..0xe4, 0xe4..0xe8] {
            assert!(zero(range.clone()), "{range:#x?}");
        }
        assert_eq!(seen.bytes[0x78..0x7c], 1u32.to_ne_bytes(), "combined");
        assert_eq!(
            seen.bytes[0xac..0xb4],
            [1u32.to_ne_bytes(), 43_780u32.to_ne_bytes()].concat(),
            "frame counter"
        );
    }

    /// Inside a callback the raw gaze calls are refused before their
    /// arguments are read, as wherever a device handle is taken, and
    /// subscribe nothing.
    #[test]
    fn raw_gaze_is_refused_inside_a_callback() {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(0, vec![])));
        let n = ptr::null_mut();
        let callback = Some(ignore_raw as GazeRawFn);
        let mut got = [0; 5];
        crate::device::call(|| {
            // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed
            // once below; a null device is allowed.
            got = unsafe {
                [
                    tobii_gaze_raw_subscribe(d, callback, n),
                    tobii_gaze_raw_subscribe(n.cast(), callback, n),
                    tobii_gaze_raw_subscribe(d, None, n),
                    tobii_gaze_raw_unsubscribe(d),
                    tobii_gaze_raw_unsubscribe(n.cast()),
                ]
            };
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 5]);
        // SAFETY: as above; the callback guard is down again.
        unsafe {
            assert_eq!(tobii_gaze_raw_unsubscribe(d), TOBII_ERROR_NOT_SUBSCRIBED);
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    /// The daemon fails every request, as with no tracker plugged in: the
    /// answer is local.
    #[test]
    fn only_eyeball_centres_are_a_supported_internal_capability() {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(
            status::CONNECTION_FAILED,
            vec![],
        )));
        let mut s = 9u32;
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `s` is a live local.
        unsafe {
            assert_eq!(tobii_internal_capability_supported(d, 0, &raw mut s), 0);
            assert_eq!(s, TOBII_SUPPORTED);
            for capability in (1..=5).chain([6, 1000, i32::MAX]) {
                s = 9;
                assert_eq!(
                    tobii_internal_capability_supported(d, capability, &raw mut s),
                    0,
                    "{capability}: unknown is not an error"
                );
                assert_eq!(s, TOBII_NOT_SUPPORTED, "{capability}");
            }
            for capability in [-1, i32::MIN] {
                s = 9;
                assert_eq!(
                    tobii_internal_capability_supported(d, capability, &raw mut s),
                    TOBII_ERROR_INVALID_PARAMETER,
                    "{capability}"
                );
                assert_eq!(s, 9, "{capability}: nothing written");
            }
            assert_eq!(
                tobii_internal_capability_supported(d, 0, ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_internal_capability_supported(d, -1, ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_internal_capability_supported(ptr::null_mut(), 0, &raw mut s),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(s, 9, "nothing written");
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    /// From inside a callback the query is refused before its arguments are
    /// read, as wherever a device handle is taken, and writes nothing.
    #[test]
    fn internal_capability_support_is_refused_inside_a_callback() {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(0, vec![])));
        let mut s = 9u32;
        let mut got = [0; 4];
        crate::device::call(|| {
            // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed
            // once below; `s` is a live local.
            got = unsafe {
                [
                    tobii_internal_capability_supported(d, 0, &raw mut s),
                    tobii_internal_capability_supported(ptr::null_mut(), 0, &raw mut s),
                    tobii_internal_capability_supported(d, 0, ptr::null_mut()),
                    tobii_internal_capability_supported(d, -1, &raw mut s),
                ]
            };
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 4]);
        assert_eq!(s, 9, "nothing written");
        // SAFETY: as above; the callback guard is down again.
        unsafe { assert_eq!(tobii_device_destroy(d), 0) };
    }

    /// Ask a daemon answering `status`/`payload` for a clock pair, into a
    /// sentinel.
    fn timesync_with(status: u8, payload: Vec<u8>) -> (Status, TimesyncData) {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(status, payload)));
        let mut out = TimesyncData {
            system_start_us: -1,
            system_end_us: -2,
            tracker_us: -3,
        };
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `out` is a live local.
        unsafe {
            let got = tobii_timesync(d, &raw mut out);
            assert_eq!(tobii_device_destroy(d), 0);
            (got, out)
        }
    }

    #[test]
    fn timesync_puts_the_daemon_pair_in_stream_engine_order() {
        let payload = encode_timesync(&Timesync {
            host_start_us: 12_000_000_000,
            device_us: 5_000_000,
            host_end_us: 12_000_030_000,
        });
        assert_eq!(
            timesync_with(0, payload),
            (
                TOBII_ERROR_NO_ERROR,
                TimesyncData {
                    system_start_us: 12_000_000_000,
                    system_end_us: 12_000_030_000,
                    tracker_us: 5_000_000,
                }
            )
        );
    }

    #[test]
    fn timesync_writes_nothing_on_an_error() {
        let untouched = TimesyncData {
            system_start_us: -1,
            system_end_us: -2,
            tracker_us: -3,
        };
        assert_eq!(
            timesync_with(status::NOT_AVAILABLE, vec![]),
            (TOBII_ERROR_NOT_AVAILABLE, untouched),
            "the daemon's status passes through"
        );
        assert_eq!(
            timesync_with(status::CONNECTION_FAILED, vec![]),
            (TOBII_ERROR_CONNECTION_FAILED, untouched),
            "no tracker"
        );
        assert_eq!(
            timesync_with(0, vec![1, 2, 3]),
            (TOBII_ERROR_INTERNAL, untouched),
            "a malformed reply"
        );

        let d = Box::into_raw(Box::new(crate::device::tests::device_with(0, vec![])));
        let mut out = untouched;
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `out` is a live local.
        unsafe {
            assert_eq!(
                tobii_timesync(d, ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_timesync(ptr::null_mut(), &raw mut out),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }
        assert_eq!(out, untouched);
    }

    /// What a stream-type receiver saw, and the device it may call back into.
    #[derive(Default)]
    struct Seen {
        entries: Vec<(i32, u32, String, String)>,
        reentry: Vec<Status>,
        device: Option<*mut Device>,
    }

    unsafe extern "C" fn collect_type(t: *const StreamType, ud: *mut c_void) {
        // SAFETY: the tests pass `&raw mut Seen` as `ud`, and the library a
        // valid entry as `t`.
        let (seen, t) = unsafe { (&mut *ud.cast::<Seen>(), &*t) };
        let text = |s: &[std::ffi::c_char; 64]| {
            // SAFETY: the library NUL-terminates both strings.
            unsafe { CStr::from_ptr(s.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        };
        seen.entries
            .push((t.type_, t.value, text(&t.name), text(&t.text)));
        if let Some(d) = seen.device {
            let mut fou = 9;
            // SAFETY: `d` is the live handle the enumeration runs on, and
            // the library no longer borrows it while the receiver runs.
            seen.reentry
                .push(unsafe { tobii_get_field_of_use(d, &raw mut fou) });
            // A call that takes the device's command lock, which would wait
            // forever were the enumeration still holding it.
            // SAFETY: as above; `ignore_type` takes any entry.
            seen.reentry.push(unsafe {
                tobii_enumerate_stream_types(d, Some(ignore_type), ptr::null_mut())
            });
        }
    }

    /// A receiver that takes each entry and does nothing with it.
    unsafe extern "C" fn ignore_type(_: *const StreamType, _: *mut c_void) {}

    /// Run `f` on a thread of its own and wait up to 10 s for what it
    /// returns: a receiver that deadlocks fails its test instead of hanging
    /// it.
    fn within_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the enumeration returned, holding no lock while its receiver ran")
    }

    /// The catalogue in the init fixture, as the daemon would send it.
    fn fixture_catalogue() -> Vec<u8> {
        let hex = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tobii-proto/fixtures/init-rsp-1200.hex"
        ));
        let bytes = tobii_proto::protocol::hex_to_bytes(hex).expect("hex");
        let msg = tobii_proto::protocol::parse_message(&bytes).expect("msg");
        encode_stream_types(&tobii_proto::facts::parse_stream_catalogue(&msg))
    }

    /// Enumerate from a daemon answering `status`/`payload`.
    fn enumerate_with(status: u8, payload: Vec<u8>, reenter: bool) -> (Status, Seen) {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(status, payload)));
        let mut seen = Seen {
            device: reenter.then_some(d),
            ..Seen::default()
        };
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `seen` is a live local.
        unsafe {
            let got = tobii_enumerate_stream_types(
                d,
                Some(collect_type),
                (&raw mut seen).cast::<c_void>(),
            );
            assert_eq!(tobii_device_destroy(d), 0);
            (got, seen)
        }
    }

    #[test]
    fn stream_types_come_in_the_trackers_order_with_the_dlls_types() {
        let (got, seen) = enumerate_with(0, fixture_catalogue(), false);

        assert_eq!(got, TOBII_ERROR_NO_ERROR);
        let types: Vec<i32> = seen.entries.iter().map(|e| e.0).collect();
        assert_eq!(types, [1, 2, 4, 11, 0, 7, 0, 0, 0]);
        let values: Vec<u32> = seen.entries.iter().map(|e| e.1).collect();
        assert_eq!(values, [0, 0, 0, 1000, 0, 0, 0, 0, 0]);
        assert_eq!(
            seen.entries[4],
            (0, 0, "primary_camera_image".into(), String::new())
        );
    }

    #[test]
    fn stream_type_strings_are_cut_to_63_bytes() {
        let payload = encode_stream_types(&[request::StreamType {
            id: 0x500,
            name: "n".repeat(80),
            text: "t".repeat(64),
            value: 0,
        }]);
        let (got, seen) = enumerate_with(0, payload, false);
        assert_eq!(got, TOBII_ERROR_NO_ERROR);
        assert_eq!(seen.entries[0].2, "n".repeat(63));
        assert_eq!(seen.entries[0].3, "t".repeat(63));
    }

    /// The receiver runs with no callback guard and no lock held: from each
    /// of the 9 entries it reads the field of use and enumerates the stream
    /// types again, which takes the device's command lock.
    #[test]
    fn a_stream_type_receiver_may_call_back_into_the_library() {
        let (got, reentry) = within_seconds(|| {
            let (got, seen) = enumerate_with(0, fixture_catalogue(), true);
            (got, seen.reentry)
        });
        assert_eq!(got, TOBII_ERROR_NO_ERROR);
        assert_eq!(reentry, [TOBII_ERROR_NO_ERROR; 18]);
    }

    #[test]
    fn stream_type_failures_call_nothing() {
        let (got, seen) = enumerate_with(status::TIMED_OUT, vec![], false);
        assert_eq!(got, crate::status::TOBII_ERROR_TIMED_OUT, "passed through");
        assert!(seen.entries.is_empty());
        let (got, seen) = enumerate_with(0, vec![9, 0, 0, 0], false);
        assert_eq!(got, TOBII_ERROR_INTERNAL, "a malformed reply");
        assert!(seen.entries.is_empty());

        let d = Box::into_raw(Box::new(crate::device::tests::device_with(0, vec![])));
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below.
        unsafe {
            assert_eq!(
                tobii_enumerate_stream_types(d, None, ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_enumerate_stream_types(ptr::null_mut(), Some(collect_type), ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    /// What a column receiver saw: each record, and what its calls back into
    /// the device answered.
    #[derive(Default)]
    struct ColumnsSeen {
        records: Vec<StreamTypeColumn>,
        reentry: Vec<Status>,
        device: Option<*mut Device>,
    }

    unsafe extern "C" fn collect_column(c: *const StreamTypeColumn, ud: *mut c_void) {
        // SAFETY: the tests pass `&raw mut ColumnsSeen` as `ud`, and the
        // library a valid record as `c`.
        let (seen, c) = unsafe { (&mut *ud.cast::<ColumnsSeen>(), *c) };
        seen.records.push(c);
        if let Some(d) = seen.device {
            let mut fou = 9;
            // SAFETY: `d` is the live handle the enumeration runs on, and
            // the library holds none of its locks while the receiver runs.
            seen.reentry
                .push(unsafe { tobii_get_field_of_use(d, &raw mut fou) });
            // A call that takes the device's command lock, which would wait
            // forever were the enumeration still holding it.
            // SAFETY: as above; `ignore_column` takes any record.
            seen.reentry.push(unsafe {
                tobii_enumerate_stream_type_columns(d, 2, Some(ignore_column), ptr::null_mut())
            });
        }
    }

    /// A receiver for calls that must not reach it.
    unsafe extern "C" fn ignore_column(_: *const StreamTypeColumn, _: *mut c_void) {}

    /// A record's string, checked to be zero after its NUL.
    fn zeroed_text(field: &[c_char]) -> String {
        let (bytes, zeroed) = c_field(field);
        assert!(zeroed, "zero after the NUL");
        String::from_utf8(bytes).expect("UTF-8")
    }

    /// A record's key, numbers, name and text.
    fn column_fields(c: &StreamTypeColumn) -> (u32, u32, u32, String, String) {
        let (name, text) = (zeroed_text(&c.name), zeroed_text(&c.text));
        (c.key, c.param_a, c.param_b, name, text)
    }

    /// The ET5's answer for its image stream, 0x501 (stream-probe,
    /// 2026-10-13).
    fn image_columns() -> Vec<request::StreamColumn> {
        let names = ["Timestamp", "Bits per pixel", "Width", "Height", "Image"];
        (1..)
            .zip(names)
            .map(|(key, name)| request::StreamColumn {
                key,
                name: name.into(),
                param_b: 1,
                ..request::StreamColumn::default()
            })
            .collect()
    }

    /// `n` columns keyed 0 to `n - 1`, with empty strings.
    fn numbered_columns(n: u32) -> Vec<u8> {
        let columns: Vec<request::StreamColumn> = (0..n)
            .map(|key| request::StreamColumn {
                key,
                ..request::StreamColumn::default()
            })
            .collect();
        encode_stream_columns(&columns)
    }

    /// The tracker's answer in the columns fixture (0x1770's), as the
    /// daemon would send it.
    fn fixture_columns() -> Vec<u8> {
        let hex = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tobii-proto/fixtures/columns-rsp-1210-0x1770.hex"
        ));
        let bytes = tobii_proto::protocol::hex_to_bytes(hex).expect("hex");
        let msg = tobii_proto::protocol::parse_message(&bytes).expect("msg");
        let columns = tobii_proto::facts::stream_columns_answer(&msg).expect("an answer");
        encode_stream_columns(&columns)
    }

    /// Enumerate the columns of `stream_type` from a daemon answering
    /// `status`/`payload`.
    fn columns_with(
        stream_type: u32,
        status: u8,
        payload: Vec<u8>,
        reenter: bool,
    ) -> (Status, ColumnsSeen) {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(status, payload)));
        let mut seen = ColumnsSeen {
            device: reenter.then_some(d),
            ..ColumnsSeen::default()
        };
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `seen` is a live local.
        unsafe {
            let got = tobii_enumerate_stream_type_columns(
                d,
                stream_type,
                Some(collect_column),
                (&raw mut seen).cast::<c_void>(),
            );
            assert_eq!(tobii_device_destroy(d), 0);
            (got, seen)
        }
    }

    /// Each column comes in the tracker's order, every field in its slot of
    /// the DLL's record and each string zero after its NUL.
    #[test]
    fn stream_columns_come_in_the_trackers_order_with_their_keys_and_numbers() {
        let (got, seen) = columns_with(2, 0, encode_stream_columns(&image_columns()), false);
        assert_eq!(got, TOBII_ERROR_NO_ERROR);
        let columns: Vec<_> = seen.records.iter().map(column_fields).collect();
        assert_eq!(
            columns,
            [
                (1, 0, 1, String::from("Timestamp"), String::new()),
                (2, 0, 1, String::from("Bits per pixel"), String::new()),
                (3, 0, 1, String::from("Width"), String::new()),
                (4, 0, 1, String::from("Height"), String::new()),
                (5, 0, 1, String::from("Image"), String::new()),
            ]
        );

        let (got, seen) = columns_with(7, 0, fixture_columns(), false);
        assert_eq!(got, TOBII_ERROR_NO_ERROR, "the ET5's answer for 0x1770");
        let columns: Vec<_> = seen.records.iter().map(column_fields).collect();
        assert_eq!(
            columns,
            [(1, 0, 1, String::from("Debug data"), String::new())]
        );

        // No two fields alike, so none can stand in another's slot.
        let odd = request::StreamColumn {
            key: 0x25,
            name: "Left User 3D Position Guide".into(),
            text: "text".into(),
            param_a: 7,
            param_b: u32::MAX,
        };
        let (got, seen) = columns_with(1, 0, encode_stream_columns(&[odd]), false);
        assert_eq!(got, TOBII_ERROR_NO_ERROR);
        let columns: Vec<_> = seen.records.iter().map(column_fields).collect();
        assert_eq!(
            columns,
            [(
                0x25,
                7,
                u32::MAX,
                String::from("Left User 3D Position Guide"),
                String::from("text")
            )]
        );
    }

    #[test]
    fn stream_column_strings_are_cut_to_63_bytes() {
        let payload = encode_stream_columns(&[request::StreamColumn {
            key: 1,
            name: "n".repeat(80),
            text: "t".repeat(64),
            ..request::StreamColumn::default()
        }]);
        let (got, seen) = columns_with(1, 0, payload, false);
        assert_eq!(got, TOBII_ERROR_NO_ERROR);
        let (_, _, _, name, text) = column_fields(&seen.records[0]);
        assert_eq!((name, text), ("n".repeat(63), "t".repeat(63)));
    }

    /// The receiver runs with no callback guard and no lock held: from each
    /// of the 5 records it reads the field of use and enumerates the columns
    /// again, which takes the device's command lock.
    #[test]
    fn a_column_receiver_may_call_back_into_the_library() {
        let (got, reentry) = within_seconds(|| {
            let (got, seen) = columns_with(2, 0, encode_stream_columns(&image_columns()), true);
            (got, seen.reentry)
        });
        assert_eq!(got, TOBII_ERROR_NO_ERROR);
        assert_eq!(reentry, [TOBII_ERROR_NO_ERROR; 10]);
    }

    /// A failed request answers its own error, a reply that does not decode
    /// is `TOBII_ERROR_INTERNAL`, and more columns than the DLL's arena
    /// holds is `TOBII_ERROR_CONNECTION_FAILED`, as in the DLL: none calls
    /// the receiver.
    #[test]
    fn stream_column_failures_call_nothing() {
        for (reply, expected) in [
            // A stream the init did not ask about or the tracker refused, or
            // a daemon older than this library.
            (status::NOT_SUPPORTED, TOBII_ERROR_NOT_SUPPORTED),
            // No tracker seen yet.
            (status::TIMED_OUT, crate::status::TOBII_ERROR_TIMED_OUT),
        ] {
            let (got, seen) = columns_with(1, reply, vec![], false);
            assert_eq!(got, expected, "reply status {reply}, passed through");
            assert!(seen.records.is_empty());
        }
        let (got, seen) = columns_with(1, 0, vec![9, 0, 0, 0], false);
        assert_eq!(got, TOBII_ERROR_INTERNAL, "a malformed reply");
        assert!(seen.records.is_empty());

        let (got, seen) = columns_with(1, 0, numbered_columns(74), false);
        assert_eq!(got, TOBII_ERROR_CONNECTION_FAILED, "74 columns");
        assert!(seen.records.is_empty());
        let (got, seen) = columns_with(1, 0, numbered_columns(73), false);
        assert_eq!(got, TOBII_ERROR_NO_ERROR, "73 columns");
        let keys: Vec<u32> = seen.records.iter().map(|c| c.key).collect();
        assert_eq!(keys, (0..73).collect::<Vec<_>>());
    }

    /// Each type asks the daemon for the columns of the device stream the
    /// DLL's map names, in a `u32`, whether or not the tracker lists it;
    /// type 10 asks nothing. An answer that lists no columns is one, with no
    /// call.
    #[test]
    fn a_type_asks_for_the_columns_of_the_stream_the_dlls_map_names() {
        use std::sync::Mutex;
        use tobii_ipc::encode_reply;
        use tobii_ipc::request::decode_request;
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        let connect = crate::device::tests::fake_daemon(move |body| {
            let req = decode_request(body).expect("request");
            log.lock()
                .expect("log")
                .push((req.kind, req.payload.to_vec()));
            vec![encode_reply(req.id, 0, &encode_stream_columns(&[]))]
        });
        let d = Box::into_raw(Box::new(Device::new(connect, 1, 1).expect("device")));
        let mut seen = ColumnsSeen::default();
        let mut got = Vec::new();
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `seen` is a live local.
        unsafe {
            for stream_type in [1, 2, 4, 7, 11, 12, 0, 14, u32::MAX, 10] {
                got.push(tobii_enumerate_stream_type_columns(
                    d,
                    stream_type,
                    Some(collect_column),
                    (&raw mut seen).cast::<c_void>(),
                ));
            }
            assert_eq!(tobii_device_destroy(d), 0);
        }

        assert_eq!(got[..9], [TOBII_ERROR_NO_ERROR; 9]);
        assert_eq!(got[9], TOBII_ERROR_NOT_SUPPORTED, "type 10");
        let ids = [0x500, 0x501, 0x504, 0x1770, 0x508, 0x1770, 0, 0, 0];
        let requests: Vec<(u8, Vec<u8>)> = ids
            .into_iter()
            .map(|id| (kind::STREAM_COLUMNS, encode_u32(id)))
            .collect();
        assert_eq!(*asked.lock().expect("log"), requests);
        assert!(seen.records.is_empty(), "no column listed, no call");
    }

    /// Type 10 is refused before the daemon is asked, as the DLL refuses it
    /// before its map, and so are a null device and a null receiver.
    #[test]
    fn stream_columns_refuse_type_10_and_null_arguments_without_asking_the_daemon() {
        let (d, frames) = crate::device::tests::counting_device();
        let mut seen = ColumnsSeen::default();
        let ud = (&raw mut seen).cast::<c_void>();
        let receiver = Some(collect_column as StreamTypeColumnReceiver);
        // SAFETY: `d` is a live handle, destroyed by `destroy_untouched`;
        // `seen` is a live local; a null device is allowed.
        let got = unsafe {
            [
                tobii_enumerate_stream_type_columns(d, 10, receiver, ud),
                tobii_enumerate_stream_type_columns(d, 1, None, ud),
                tobii_enumerate_stream_type_columns(ptr::null_mut(), 1, receiver, ud),
            ]
        };
        assert_eq!(
            got,
            [
                TOBII_ERROR_NOT_SUPPORTED,
                TOBII_ERROR_INVALID_PARAMETER,
                TOBII_ERROR_INVALID_PARAMETER
            ]
        );
        assert!(seen.records.is_empty());
        crate::device::tests::destroy_untouched(d, &frames);
    }

    /// Inside a callback every call is refused before its arguments are
    /// read, as wherever a device handle is taken, and asks nothing. The DLL
    /// checks the nulls first, so for those it would answer
    /// `TOBII_ERROR_INVALID_PARAMETER`.
    #[test]
    fn stream_columns_are_refused_inside_a_callback() {
        let (d, frames) = crate::device::tests::counting_device();
        let n = ptr::null_mut();
        let receiver = Some(ignore_column as StreamTypeColumnReceiver);
        let mut got = [0; 4];
        crate::device::call(|| {
            // SAFETY: `d` is a live handle, destroyed by `destroy_untouched`;
            // a null device is allowed.
            got = unsafe {
                [
                    tobii_enumerate_stream_type_columns(d, 1, receiver, n),
                    tobii_enumerate_stream_type_columns(n.cast(), 1, receiver, n),
                    tobii_enumerate_stream_type_columns(d, 1, None, n),
                    tobii_enumerate_stream_type_columns(d, 10, receiver, n),
                ]
            };
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 4]);
        crate::device::tests::destroy_untouched(d, &frames);
    }

    #[test]
    fn pause_and_resume_ask_the_daemon() {
        use std::sync::{Arc, Mutex};
        use tobii_ipc::encode_reply;
        use tobii_ipc::request::decode_request;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let connect = crate::device::tests::fake_daemon(move |body| {
            let req = decode_request(body).expect("request");
            log.lock()
                .expect("log")
                .push((req.kind, req.payload.to_vec()));
            vec![encode_reply(req.id, 0, &[])]
        });
        let d = Box::into_raw(Box::new(Device::new(connect, 1, 1).expect("device")));
        // SAFETY: `d` is live and destroyed once; a null device is allowed.
        unsafe {
            assert_eq!(tobii_pause_device(d), TOBII_ERROR_NO_ERROR);
            assert_eq!(tobii_resume_device(d), TOBII_ERROR_NO_ERROR);
            assert_eq!(tobii_device_destroy(d), 0);
            assert_eq!(
                tobii_pause_device(ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_resume_device(ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
        }
        assert_eq!(
            *seen.lock().expect("log"),
            vec![(kind::DEVICE_PAUSE, vec![1]), (kind::DEVICE_PAUSE, vec![0])]
        );
    }

    #[test]
    fn pause_and_resume_carry_the_daemon_status() {
        // The daemon says a calibration session runs.
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(
            status::CALIBRATION_BUSY,
            vec![],
        )));
        // SAFETY: `d` is live and destroyed once.
        unsafe {
            assert_eq!(
                tobii_pause_device(d),
                crate::status::TOBII_ERROR_CALIBRATION_BUSY
            );
            assert_eq!(
                tobii_resume_device(d),
                crate::status::TOBII_ERROR_CALIBRATION_BUSY
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }
    }

    /// A configuration with every field set, to see what a call writes.
    fn sentinel_configuration() -> HardwareConfiguration {
        let entry = HardwareConfigurationEntry {
            id: -7,
            coefficient_count: -7,
            coefficients: [-7.0; 64],
            param_d: -7.0,
            ..NO_HARDWARE_ENTRY
        };
        HardwareConfiguration {
            entry_count: -7,
            entries: [entry; 2],
            point_count: -7,
            points_xyz: [[-7.0; 3]; 40],
            mode: -7,
        }
    }

    /// Ask a daemon answering `status`/`payload` for the hardware
    /// configuration, into a sentinel.
    fn hardware_with(status: u8, payload: Vec<u8>) -> (Status, HardwareConfiguration) {
        let d = Box::into_raw(Box::new(crate::device::tests::device_with(status, payload)));
        let mut out = sentinel_configuration();
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `out` is a live local.
        unsafe {
            let got = tobii_hardware_configuration_get(d, &raw mut out);
            assert_eq!(tobii_device_destroy(d), 0);
            (got, out)
        }
    }

    #[test]
    #[allow(clippy::float_cmp)] // reason: values copied, not computed
    fn the_hardware_configuration_fills_the_dll_layout() {
        let h = request::HardwareConfiguration {
            entries: vec![request::HardwareEntry {
                id: u32::MAX,
                param_a: 16.0,
                param_b: 100.0,
                position_mm: [0.0, 0.0, 4.14],
                values: [0.5; 15],
                width: 2240,
                height: 2241,
                param_c: 3,
                coefficients: vec![1.0, -0.5],
                point_a_mm: [1.0, 2.0, 3.0],
                point_b_mm: [4.0, 5.0, 6.0],
                param_d: 0.25,
            }],
            points_mm: vec![[130.0, 0.76, 1.62], [-130.0, 0.76, 1.62]],
            mode: 2,
        };

        let (got, c) = hardware_with(0, request::encode_hardware_configuration(&h));

        assert_eq!(got, TOBII_ERROR_NO_ERROR);
        assert_eq!((c.entry_count, c.point_count, c.mode), (1, 2, 2));
        let e = &c.entries[0];
        assert_eq!((e.id, e.param_a, e.param_b), (-1, 16.0, 100.0));
        assert_eq!((e.position_xyz, e.values), ([0.0, 0.0, 4.14], [0.5; 15]));
        assert_eq!((e.width, e.height, e.param_c), (2240, 2241, 3));
        assert_eq!(e.coefficient_count, 2);
        assert_eq!(e.coefficients[..3], [1.0, -0.5, 0.0], "zero past the count");
        assert_eq!(
            (e.point_a_xyz, e.point_b_xyz),
            ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
        );
        assert_eq!(e.param_d, 0.25);
        assert_eq!(c.entries[1], NO_HARDWARE_ENTRY, "zero past the count");
        assert_eq!(c.points_xyz[1], [-130.0, 0.76, 1.62]);
        assert_eq!(c.points_xyz[2], [0.0; 3], "zero past the count");
    }

    #[test]
    fn a_hardware_mode_outside_the_dlls_range_is_zero() {
        for (mode, want) in [(0, 0), (1, 1), (2, 2), (3, 0), (u32::MAX, 0)] {
            let h = request::HardwareConfiguration {
                mode,
                ..request::HardwareConfiguration::default()
            };
            let (got, c) = hardware_with(0, request::encode_hardware_configuration(&h));
            assert_eq!((got, c.mode), (TOBII_ERROR_NO_ERROR, want), "{mode}");
        }
    }

    #[test]
    fn a_hardware_configuration_failure_writes_nothing() {
        let (got, c) = hardware_with(status::NOT_SUPPORTED, vec![]);
        assert_eq!(
            got,
            crate::status::TOBII_ERROR_NOT_SUPPORTED,
            "what the ET5 answers on Linux, passed through"
        );
        assert_eq!(c, sentinel_configuration());
        let (got, c) = hardware_with(0, vec![3]);
        assert_eq!(got, TOBII_ERROR_INTERNAL, "a malformed reply");
        assert_eq!(c, sentinel_configuration());

        let d = Box::into_raw(Box::new(crate::device::tests::device_with(0, vec![])));
        let mut out = sentinel_configuration();
        // SAFETY: `d` is a live handle from `Box::into_raw`, destroyed once
        // below; `out` is a live local.
        unsafe {
            assert_eq!(
                tobii_hardware_configuration_get(d, ptr::null_mut()),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(
                tobii_hardware_configuration_get(ptr::null_mut(), &raw mut out),
                TOBII_ERROR_INVALID_PARAMETER
            );
            assert_eq!(tobii_device_destroy(d), 0);
        }
        assert_eq!(out, sentinel_configuration());
    }

    #[test]
    fn stream_ids_map_as_in_the_dll() {
        let ids = [
            (0x500, 1),
            (0x501, 2),
            (0x502, 3),
            (0x503, 14),
            (0x504, 4),
            (0x505, 5),
            (0x506, 8),
            (0x507, 9),
            (0x508, 11),
            (0x509, 0),
            (0x50a, 6),
            (0x1770, 7),
            (0x50e, 0),
            (0x1771, 0),
            (0x1774, 0),
            (0, 0),
        ];
        for (id, want) in ids {
            assert_eq!(se_stream_type(id), want, "{id:#x}");
        }
    }

    #[test]
    fn column_types_map_as_in_the_dll() {
        let types = [
            (0, 0),
            (1, 0x500),
            (2, 0x501),
            (3, 0x502),
            (4, 0x504),
            (5, 0x505),
            (6, 0x50a),
            (7, 0x1770),
            (8, 0x506),
            (9, 0x507),
            // Refused before the map, so never asked about.
            (10, 0),
            (11, 0x508),
            (12, 0x1770),
            (13, 0),
            (14, 0),
            (15, 0),
            (u32::MAX, 0),
        ];
        for (stream_type, want) in types {
            assert_eq!(columns_stream_id(stream_type), want, "{stream_type}");
        }
        // Not the catalogue's map run backwards: 12 asks for 0x1770, which
        // the catalogue lists as 7, and it lists 0x503 as 14, which asks for
        // 0.
        assert_eq!(se_stream_type(columns_stream_id(12)), 7);
        assert_eq!((se_stream_type(0x503), columns_stream_id(14)), (14, 0));
    }

    /// The streams an init asks the columns of are every device stream a
    /// type can ask about, each once, in numeric order.
    #[test]
    fn the_column_stream_ids_are_every_stream_a_type_asks_about() {
        let asked: BTreeSet<u32> = (1..=12)
            .filter(|t| *t != 10)
            .map(columns_stream_id)
            .collect();
        assert_eq!(asked, BTreeSet::from(COLUMN_STREAM_IDS));
        assert!(COLUMN_STREAM_IDS.is_sorted_by(|a, b| a < b));
        assert!(!COLUMN_STREAM_IDS.contains(&0));
    }
}
