//! How long each call that asks tobiid waits for its answer: the shared
//! table in [`tobii_ipc::timeout`], which outlasts what the daemon may take
//! over each request, so that the call reports what the daemon did. One
//! that gave up first would return `TOBII_ERROR_TIMED_OUT` while the daemon
//! carried the request out (a calibration started, a display area set), and
//! the device's next request would wait behind it, since the daemon runs a
//! connection's requests one at a time. `TOBII_ERROR_TIMED_OUT` from a
//! request is then the daemon's own answer, or a daemon held up past its
//! deadlines by what they leave out, which may still carry the request out.
//! What the daemon waits for over each request, and its worst case, are in
//! [`tobii_ipc::timeout`]'s table; here, which calls wait which timeout:
//!
//! | Timeout | Calls |
//! |---|---|
//! | [`FACTS`] 12 s | the facts reads (below), `tobii_set_device_name` |
//! | [`STATE`] 3 s | `tobii_get_state_bool`, `tobii_get_state_uint32` |
//! | [`TIMESYNC`] 27 s | `tobii_timesync` |
//! | [`DEVICE_PAUSE`] 60 s | `tobii_pause_device`, `tobii_resume_device` |
//! | [`CATALOGUE_STREAM_START`] 65 s | `tobii_custom_stream_subscribe` of stream 0, `tobii_image_collection_subscribe` |
//! | [`CATALOGUE_STREAM_STOP`] 55 s | `tobii_custom_stream_unsubscribe` of stream 0, `tobii_image_collection_unsubscribe` |
//! | [`DISPLAY_AREA_SET`] 75 s | `tobii_set_display_area` |
//! | [`CALIBRATION_START`] 190 s | `tobii_calibration_start` |
//! | [`CALIBRATION_STOP`] 120 s | `tobii_calibration_stop` |
//! | [`CALIBRATION_COLLECT_2D`] 40 s | `tobii_calibration_collect_data_2d` |
//! | [`CALIBRATION_DISCARD_2D`] 40 s | `tobii_calibration_discard_data_2d` |
//! | [`CALIBRATION_CLEAR`] 40 s | `tobii_calibration_clear` |
//! | [`CALIBRATION_COMPUTE`] 80 s | `tobii_calibration_compute_and_apply` |
//! | [`CALIBRATION_RETRIEVE`] 40 s | `tobii_calibration_retrieve` |
//! | [`CALIBRATION_APPLY`] 45 s | `tobii_calibration_apply` |
//!
//! The facts reads are `tobii_get_device_info`, `tobii_get_track_box`,
//! `tobii_get_display_area`, `tobii_get_geometry_mounting`,
//! `tobii_get_device_name` (before a name is set), `tobii_get_state_string`,
//! `tobii_enumerate_stream_types`, `tobii_enumerate_stream_type_columns`
//! (but for stream type 10, which asks nothing) and
//! `tobii_hardware_configuration_get`; and, once per connection (and again
//! after a fetch that failed, or while tobiid has none to give), the
//! subscribes and unsubscribes of the custom stream and image collection,
//! for the tracker's catalogue (`STREAM_TYPES`), which they ask for before
//! anything else, and `tobii_internal_stream_supported` of internal streams
//! 2 and 6, answered from the same catalogue (the IR image, 0, and every
//! other id ask nothing). The DLL answers that query from what its tracker
//! module read at connect; libtobii's may wait for the catalogue (and, as
//! any call that asks the daemon, for the round trips other threads have
//! under way on the device or queued ahead of it), though it never fails: a
//! catalogue it cannot get leaves 2 and 6 unsupported.
//! A subscription change makes no request: it waits for an acknowledgement
//! the daemon gives without the tracker, 2 s (see `device`). A reconnect
//! waits ~500 ms in all for that acknowledgement and for the daemon's
//! answers to the holds of the custom stream and image collection restored
//! (`CATALOGUE_STREAM_START` with `STREAM_START_RESTORE`), which the daemon
//! gives from its own state: less than the 3 s of
//! [`tobii_ipc::timeout::CATALOGUE_STREAM_RESTORE`], as it gives up sooner
//! on a daemon that is not running yet (see `device::RECONNECT_ACK_TIMEOUT`).
//! Each timeout is checked against its worst case at compile time (in
//! `tobii-ipc`), and that each call passes its own in the tests.

pub(crate) use tobii_ipc::timeout::{
    CALIBRATION_APPLY, CALIBRATION_CLEAR, CALIBRATION_COLLECT_2D, CALIBRATION_COMPUTE,
    CALIBRATION_DISCARD_2D, CALIBRATION_RETRIEVE, CALIBRATION_START, CALIBRATION_STOP,
    CATALOGUE_STREAM_START, CATALOGUE_STREAM_STOP, DEVICE_PAUSE, DISPLAY_AREA_SET, FACTS, STATE,
    TIMESYNC,
};

#[cfg(test)]
mod tests {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::ptr;

    use tobii_ipc::request::status;

    use super::*;
    use crate::api::{
        tobii_device_destroy, tobii_get_device_info, tobii_get_state_bool, tobii_get_state_string,
        tobii_get_state_uint32, tobii_get_track_box,
    };
    use crate::calibration::{
        tobii_calibration_apply, tobii_calibration_clear, tobii_calibration_collect_data_2d,
        tobii_calibration_compute_and_apply, tobii_calibration_discard_data_2d,
        tobii_calibration_retrieve, tobii_calibration_start, tobii_calibration_stop,
    };
    use crate::config::{
        tobii_get_device_name, tobii_get_display_area, tobii_get_geometry_mounting,
        tobii_set_device_name, tobii_set_display_area,
    };
    use crate::device::tests::{
        catalogue_daemon, catalogue_of, device_with, take_request_timeouts,
    };
    use crate::device::{CUSTOM_STREAM, Device, IMAGE_COLLECTION_STREAM};
    use crate::internal::{
        tobii_custom_stream_subscribe, tobii_custom_stream_unsubscribe,
        tobii_enumerate_stream_type_columns, tobii_enumerate_stream_types,
        tobii_hardware_configuration_get, tobii_image_collection_subscribe,
        tobii_image_collection_unsubscribe, tobii_internal_stream_supported, tobii_pause_device,
        tobii_resume_device, tobii_timesync,
    };
    use crate::types::{
        CustomStreamData, DeviceInfo, DeviceName, DisplayArea, GeometryMounting,
        HardwareConfiguration, ImageCollection, StateString, StreamType, StreamTypeColumn,
        TOBII_ENABLED_EYE_BOTH, TOBII_STATE_CALIBRATION_ACTIVE, TOBII_STATE_CALIBRATION_ID,
        TOBII_STATE_DEVICE_PAUSED, TOBII_STATE_FAULT, TimesyncData, TrackBox,
    };

    unsafe extern "C" fn ignore_blob(_: *const c_void, _: usize, _: *mut c_void) {}

    unsafe extern "C" fn ignore_stream_type(_: *const StreamType, _: *mut c_void) {}

    unsafe extern "C" fn ignore_column(_: *const StreamTypeColumn, _: *mut c_void) {}

    unsafe extern "C" fn ignore_custom(_: *const CustomStreamData, _: *mut c_void) {}

    unsafe extern "C" fn ignore_image_collection(_: *const ImageCollection, _: *mut c_void) {}

    /// Each call that asks the daemon waits for its answer as long as its
    /// row of the table says. The daemon refuses every request, so no answer
    /// needs decoding: the timeout is noted as the request goes out. Internal
    /// streams 2 and 6 each ask it for the catalogue, which its refusal
    /// leaves unkept, and the IR image asks nothing. The calls of the custom
    /// stream and image collection get as far as the stream's start and stop
    /// only from a daemon that lists the stream and takes its start: they
    /// have one of their own, which asks for the catalogue once, for the
    /// custom stream's subscribe.
    #[test]
    fn each_call_waits_for_the_daemon_as_long_as_its_row_says() {
        let d = Box::into_raw(Box::new(device_with(status::OPERATION_FAILED, vec![])));
        let listed = [IMAGE_COLLECTION_STREAM, CUSTOM_STREAM];
        let catalogue = (status::OK, catalogue_of(&listed));
        let (connect, _) = catalogue_daemon(catalogue, vec![]);
        let custom = Box::into_raw(Box::new(Device::new(connect, 1, 1).expect("device")));
        let _ = take_request_timeouts();
        let area = DisplayArea::default();
        let blob = [0u8; 4];
        let mut info = MaybeUninit::<DeviceInfo>::uninit();
        let mut track_box = MaybeUninit::<TrackBox>::uninit();
        let mut shown = MaybeUninit::<DisplayArea>::uninit();
        let mut mounting = MaybeUninit::<GeometryMounting>::uninit();
        let mut name = MaybeUninit::<DeviceName>::uninit();
        let mut text = MaybeUninit::<StateString>::uninit();
        let mut hardware = MaybeUninit::<HardwareConfiguration>::uninit();
        let mut timesync = MaybeUninit::<TimesyncData>::uninit();
        let mut value = 0u32;
        let mut got = Vec::new();
        let mut took = |call: &'static str| got.push((call, take_request_timeouts()));

        // SAFETY: `d` and `custom` are live handles from `Box::into_raw`,
        // each destroyed once at the end; every other pointer is to a live
        // local of the type the call reads or writes, and every receiver and
        // callback has the type it takes.
        unsafe {
            tobii_get_device_info(d, info.as_mut_ptr());
            took("tobii_get_device_info");
            tobii_get_track_box(d, track_box.as_mut_ptr());
            took("tobii_get_track_box");
            tobii_get_display_area(d, shown.as_mut_ptr());
            took("tobii_get_display_area");
            tobii_get_geometry_mounting(d, mounting.as_mut_ptr());
            took("tobii_get_geometry_mounting");
            tobii_get_device_name(d, name.as_mut_ptr());
            took("tobii_get_device_name");
            tobii_get_state_string(d, TOBII_STATE_FAULT, text.as_mut_ptr());
            took("tobii_get_state_string");
            tobii_enumerate_stream_types(d, Some(ignore_stream_type), ptr::null_mut());
            took("tobii_enumerate_stream_types");
            tobii_enumerate_stream_type_columns(d, 1, Some(ignore_column), ptr::null_mut());
            took("tobii_enumerate_stream_type_columns");
            for stream in [0, 2, 6] {
                tobii_internal_stream_supported(d, stream, &raw mut value);
            }
            took("tobii_internal_stream_supported");
            tobii_hardware_configuration_get(d, hardware.as_mut_ptr());
            took("tobii_hardware_configuration_get");
            tobii_set_device_name(d, c"Desk".as_ptr());
            took("tobii_set_device_name");
            tobii_get_state_bool(d, TOBII_STATE_DEVICE_PAUSED, &raw mut value);
            tobii_get_state_bool(d, TOBII_STATE_CALIBRATION_ACTIVE, &raw mut value);
            took("tobii_get_state_bool");
            tobii_get_state_uint32(d, TOBII_STATE_CALIBRATION_ID, &raw mut value);
            took("tobii_get_state_uint32");
            tobii_timesync(d, timesync.as_mut_ptr());
            took("tobii_timesync");
            tobii_pause_device(d);
            tobii_resume_device(d);
            took("tobii_pause_device, tobii_resume_device");
            let subscribed =
                tobii_custom_stream_subscribe(custom, Some(ignore_custom), 0, ptr::null_mut());
            took("tobii_custom_stream_subscribe");
            let unsubscribed = tobii_custom_stream_unsubscribe(custom, 0);
            took("tobii_custom_stream_unsubscribe");
            assert_eq!((subscribed, unsubscribed), (0, 0));
            let subscribed = tobii_image_collection_subscribe(
                custom,
                Some(ignore_image_collection),
                ptr::null_mut(),
            );
            took("tobii_image_collection_subscribe");
            let unsubscribed = tobii_image_collection_unsubscribe(custom);
            took("tobii_image_collection_unsubscribe");
            assert_eq!((subscribed, unsubscribed), (0, 0));
            tobii_set_display_area(d, &raw const area);
            took("tobii_set_display_area");
            tobii_calibration_start(d, TOBII_ENABLED_EYE_BOTH);
            took("tobii_calibration_start");
            tobii_calibration_stop(d);
            took("tobii_calibration_stop");
            tobii_calibration_collect_data_2d(d, 0.5, 0.5);
            took("tobii_calibration_collect_data_2d");
            tobii_calibration_discard_data_2d(d, 0.5, 0.5);
            took("tobii_calibration_discard_data_2d");
            tobii_calibration_clear(d);
            took("tobii_calibration_clear");
            tobii_calibration_compute_and_apply(d);
            took("tobii_calibration_compute_and_apply");
            tobii_calibration_retrieve(d, Some(ignore_blob), ptr::null_mut());
            took("tobii_calibration_retrieve");
            tobii_calibration_apply(d, blob.as_ptr().cast(), blob.len());
            took("tobii_calibration_apply");
            assert_eq!(tobii_device_destroy(d), 0);
            assert_eq!(tobii_device_destroy(custom), 0);
        }

        assert_eq!(
            got,
            [
                ("tobii_get_device_info", vec![FACTS]),
                ("tobii_get_track_box", vec![FACTS]),
                ("tobii_get_display_area", vec![FACTS]),
                ("tobii_get_geometry_mounting", vec![FACTS]),
                ("tobii_get_device_name", vec![FACTS]),
                ("tobii_get_state_string", vec![FACTS]),
                ("tobii_enumerate_stream_types", vec![FACTS]),
                ("tobii_enumerate_stream_type_columns", vec![FACTS]),
                ("tobii_internal_stream_supported", vec![FACTS, FACTS]),
                ("tobii_hardware_configuration_get", vec![FACTS]),
                ("tobii_set_device_name", vec![FACTS]),
                ("tobii_get_state_bool", vec![STATE, STATE]),
                ("tobii_get_state_uint32", vec![STATE]),
                ("tobii_timesync", vec![TIMESYNC]),
                (
                    "tobii_pause_device, tobii_resume_device",
                    vec![DEVICE_PAUSE, DEVICE_PAUSE]
                ),
                (
                    "tobii_custom_stream_subscribe",
                    vec![FACTS, CATALOGUE_STREAM_START]
                ),
                (
                    "tobii_custom_stream_unsubscribe",
                    vec![CATALOGUE_STREAM_STOP]
                ),
                (
                    "tobii_image_collection_subscribe",
                    vec![CATALOGUE_STREAM_START]
                ),
                (
                    "tobii_image_collection_unsubscribe",
                    vec![CATALOGUE_STREAM_STOP]
                ),
                ("tobii_set_display_area", vec![DISPLAY_AREA_SET]),
                ("tobii_calibration_start", vec![CALIBRATION_START]),
                ("tobii_calibration_stop", vec![CALIBRATION_STOP]),
                (
                    "tobii_calibration_collect_data_2d",
                    vec![CALIBRATION_COLLECT_2D]
                ),
                (
                    "tobii_calibration_discard_data_2d",
                    vec![CALIBRATION_DISCARD_2D]
                ),
                ("tobii_calibration_clear", vec![CALIBRATION_CLEAR]),
                (
                    "tobii_calibration_compute_and_apply",
                    vec![CALIBRATION_COMPUTE]
                ),
                ("tobii_calibration_retrieve", vec![CALIBRATION_RETRIEVE]),
                ("tobii_calibration_apply", vec![CALIBRATION_APPLY]),
            ]
        );
    }
}
