/* tobii/tobii_internal.h — the exports of tobii_stream_engine.dll 4.1.0.3
 * that the Stream Engine never documented, as provided by libtobii.so.
 *
 * Argument counts are those the DLL's code reads (tools/abi/dll_abi.py);
 * argument types are best guesses. Only the field-of-use, image, raw gaze,
 * foveated rendering gaze point, custom stream, image collection,
 * internal-stream, internal-capability, timesync, stream-type, stream-column,
 * pause, hardware-configuration and internal device-info functions are
 * implemented, and tobii_calibration_stimulus_points_get, the subscribes and
 * unsubscribes of internal streams 3, 4, 5, 7 and 8, the face id calls,
 * tobii_send_statistics and tobii_set_display_info check their arguments
 * before they answer, as the DLL does, and tobii_set_fw_upgrade_allowed
 * answers TOBII_ERROR_NOT_SUPPORTED as the DLL does, checking nothing; every
 * other entry point here returns TOBII_ERROR_NOT_SUPPORTED without reading its
 * arguments, so the guessed types cannot matter at runtime.
 * Companion to tobii/tobii.h and tobii/tobii_streams.h.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef TOBII_INTERNAL_H
#define TOBII_INTERNAL_H

#include "tobii.h"
#include "tobii_streams.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef void ( *tobii_field_of_use_callback_t )( tobii_field_of_use_t field_of_use, void* user_data );

/* The field of use the device was created with. */
TOBII_API tobii_error_t TOBII_CALL tobii_get_field_of_use( tobii_device_t* device,
    tobii_field_of_use_t* field_of_use );
/* Accepted; the callback is never called (the field of use cannot change). */
TOBII_API tobii_error_t TOBII_CALL tobii_field_of_use_subscribe( tobii_device_t* device,
    tobii_field_of_use_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_field_of_use_unsubscribe( tobii_device_t* device );

/* An IR camera frame: 280x280, 8 bits per pixel, ~33 Hz on the ET5. `data` is
 * valid only during the callback. timestamp_us is when the frame was taken,
 * on the tobii_system_clock clock, as every sample's (tobii_streams.h).
 * Layout from the DLL's image dispatcher. */
typedef struct tobii_image_t
{
    int64_t timestamp_us;
    int width;
    int padding_per_row;
    int height;
    int bits_per_pixel;
    void const* data;
} tobii_image_t;

typedef void ( *tobii_image_callback_t )( tobii_image_t const* image, void* user_data );

TOBII_API tobii_error_t TOBII_CALL tobii_image_subscribe( tobii_device_t* device,
    tobii_image_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_image_unsubscribe( tobii_device_t* device );

/* Raw gaze: the Stream Engine's own record of each gaze frame, one per frame
 * (~33 Hz on the ET5). The layout is the record the DLL's process_gaze
 * (0x18018d9a0) fills from the frame's keys, as its raw gaze dispatch copies
 * it (0x180171b81..0x180171db8; 232 bytes); the names are ours, after
 * tobii_gaze_data_t's (tobii_advanced.h) where the DLL fills those from the
 * same keys. Points are mm in the tracker frame (origin at the tracker, z
 * towards the user); gaze_origin_in_track_box_normalized_xyz (keys 0x03 and
 * 0x09, normalised as tobii_eye_position_normalized_t is, not to the track
 * box) and the display positions are normalised and unclamped;
 * pupil_diameter_mm is the value gaze data's pupil diameter comes from.
 * timestamp_tracker_us is on the tracker's clock, as gaze data's, which may
 * restart when the tracker re-initialises; there is no system time
 * (tobii_timesync pairs the clocks).
 *
 * Every value is passed as the tracker sent it, whatever its validity, a
 * lost eye's included. The eye blocks are keys 0x02..0x07 (left) and
 * 0x08..0x0d (right) in field order, and the combined gaze point key 0x1c.
 * status is the eye's status as sent: the ET5 sends 0 for a tracked eye and
 * 4 for a lost one, and the DLL's gaze data takes an eye's values as valid
 * while it is below 2. Each tobii_validity_t *_validity field says only that
 * the frame had the key (the record's flag, set by process_gaze when the key
 * is there), not that the value is usable: every ET5 frame captured has each
 * of those keys but 0x0e, so an eyeball centre (keys 0x17, 0x18) is valid
 * here while its eye is lost. A key the frame lacks reads 0, with a validity
 * of 0. key_0e and key_11 are keys 0x0e and 0x11, of unknown meaning (the
 * ET5 never sends 0x0e and sends 4 for 0x11); frame_counter is key 0x14.
 * combined_gaze_validity (key 0x1b) is no such flag but, like the left and
 * right origin flags (keys 0x16 and 0x15), the tracker's own word, 1 when
 * valid. The reserved fields, named by their offsets, are slots the record
 * never fills: always 0.
 *
 * The DLL serves raw gaze only from its in-process tracker module, which it
 * runs for any URL, this library's tobii-ffi:// included, but tobii-prp://
 * and tprp-tcp://, and only with the internal feature group:
 * TOBII_ERROR_INSUFFICIENT_LICENSE below it, and TOBII_ERROR_NOT_SUPPORTED
 * behind the Tobii service. This library checks no licence and serves it to
 * every client. A daemon older than this library acknowledges the
 * subscription and never sends the stream. */
typedef struct tobii_gaze_raw_eye_t
{
    float gaze_origin_from_eye_tracker_mm_xyz[ 3 ];
    float gaze_origin_in_track_box_normalized_xyz[ 3 ];
    float gaze_point_from_eye_tracker_mm_xyz[ 3 ];
    float gaze_point_on_display_normalized_xy[ 2 ];
    float pupil_diameter_mm;
    uint32_t status;
} tobii_gaze_raw_eye_t;

typedef struct tobii_gaze_raw_t
{
    int64_t timestamp_tracker_us;
    tobii_gaze_raw_eye_t left;
    tobii_gaze_raw_eye_t right;
    float combined_gaze_point_on_display_normalized_xy[ 2 ];
    uint32_t combined_gaze_validity;
    tobii_validity_t key_0e_validity;
    uint32_t key_0e;
    tobii_validity_t reserved_84;
    float reserved_88;
    tobii_validity_t reserved_8c;
    float reserved_90;
    tobii_validity_t key_11_validity;
    uint32_t key_11;
    tobii_validity_t reserved_9c;
    float reserved_a0;
    tobii_validity_t reserved_a4;
    float reserved_a8;
    tobii_validity_t frame_counter_validity;
    uint32_t frame_counter;
    tobii_validity_t left_origin_flag_validity;
    uint32_t left_origin_flag;
    tobii_validity_t right_origin_flag_validity;
    uint32_t right_origin_flag;
    tobii_validity_t left_eyeball_center_validity;
    float left_eyeball_center_from_eye_tracker_mm_xyz[ 3 ];
    tobii_validity_t right_eyeball_center_validity;
    float right_eyeball_center_from_eye_tracker_mm_xyz[ 3 ];
    uint32_t reserved_e4;
} tobii_gaze_raw_t;

typedef void ( *tobii_gaze_raw_callback_t )( tobii_gaze_raw_t const* gaze_raw, void* user_data );

TOBII_API tobii_error_t TOBII_CALL tobii_gaze_raw_subscribe( tobii_device_t* device,
    tobii_gaze_raw_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_gaze_raw_unsubscribe( tobii_device_t* device );

/* The foveated rendering gaze point, which the DLL serves for a tracker it
 * drives itself (legacy TTP) whatever the licence, from the gaze frames raw
 * gaze comes from: one tobii_gaze_point_t per raw gaze record, after raw
 * gaze's callback for it. Its position is the mean of the display-normalised
 * gaze points of the eyes whose status is below 2, or the one such eye's,
 * held still until a point lies more than 0.02 from it; it starts at
 * FLT_MAX on both axes and is kept across unsubscribes and reconnects. The
 * validity says whether either eye counted, and the timestamp is the frame's
 * on the host clock (tobii_system_clock's), as the DLL stamps it, where raw
 * gaze keeps the tracker's; a tobiid from before it sends raw gaze without
 * the host time, and the timestamp is then the tracker's. Subscribing asks
 * tobiid for the raw gaze stream. Behind Tobii's service the DLL answers
 * TOBII_ERROR_NOT_SUPPORTED. The callback type's name is ours. The subscribe
 * took void const* for the callback before; the ABI is the same. In C a
 * callback of another function type now gets an incompatible-pointer-types
 * diagnostic (an error by default since GCC 14), but one held in a void
 * pointer still converts silently, and it is now called: it must have the
 * declared signature. In C++ both are errors. */
typedef void ( *tobii_foveated_rendering_gaze_point_callback_t )( tobii_gaze_point_t const* gaze_point,
    void* user_data );

TOBII_API tobii_error_t TOBII_CALL tobii_foveated_rendering_gaze_point_subscribe( tobii_device_t* device,
    tobii_foveated_rendering_gaze_point_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_foveated_rendering_gaze_point_unsubscribe( tobii_device_t* device );

/* The custom stream, internal stream 2. Custom stream 0 gets the frames of
 * the tracker's stream 0x1770 (algodbg in its catalogue), one record each:
 * stream is 0, reserved_04 0, and data points to size bytes, the frame's key
 * 1 ("Debug data", the one column the tracker lists for the stream), valid
 * during the callback only, or is NULL when size is 0. The ET5 sends some
 * 133 frames a second, four for each gaze frame, every one with key 1 empty,
 * and each wakes tobii_wait_for_callbacks, as any sample does. The layout is
 * the DLL's (24 bytes, its dispatch's record at 0x180154666..0x18015467f);
 * the names are ours.
 *
 * The answers come as in the DLL: TOBII_ERROR_CALLBACK_IN_PROGRESS inside a
 * callback (checked first here), TOBII_ERROR_INVALID_PARAMETER for a null
 * device or callback, TOBII_ERROR_NOT_SUPPORTED when the tracker's catalogue
 * does not list 0x1770 (before the rest, TOBII_ERROR_NOT_SUBSCRIBED too),
 * TOBII_ERROR_ALREADY_SUBSCRIBED for a stream subscribed already, and
 * TOBII_ERROR_TOO_MANY_SUBSCRIBERS once 256 are. A stream other than 0 is
 * TOBII_ERROR_NOT_SUPPORTED while stream 0 is not subscribed, and otherwise
 * rides on it, TOBII_ERROR_NO_ERROR, its callback never called; its
 * unsubscribe is TOBII_ERROR_NOT_SUPPORTED while stream 0 is subscribed and
 * TOBII_ERROR_NO_ERROR once it is not. Stream 0's subscribe has tobiid hold
 * the stream for the connection, which the tracker then runs (its command
 * 1220), and answers what came of that as the DLL maps the tracker's result:
 * TOBII_ERROR_CONNECTION_FAILED_DRIVER when the tracker refused it with
 * 0x20000505 or an error the DLL has no other answer for, was lost meanwhile
 * or did not answer in time, TOBII_ERROR_INTERNAL when it refused it with
 * 0x20000508, and otherwise as the refusal says: TOBII_ERROR_NOT_SUPPORTED,
 * TOBII_ERROR_INVALID_PARAMETER or TOBII_ERROR_OPERATION_FAILED; it waits
 * ~65 s at most for the start. Its unsubscribe answers TOBII_ERROR_NO_ERROR
 * once the subscription is gone, whatever the stop (1230) came to, which it
 * waits ~55 s at most for; a stop tobiid refuses (another client's start or
 * stop held it up) is sent again without a wait, and should tobiid refuse
 * that too, it holds the stream, whose frames then reach no callback, until
 * the connection closes or stream 0 is subscribed and unsubscribed again.
 * Either call may first wait ~12 s for the catalogue, once per connection,
 * and a fetch that fails answers its error (TOBII_ERROR_TIMED_OUT,
 * TOBII_ERROR_CONNECTION_FAILED), but for an unsubscribe of a stream
 * subscribed, which goes on as the DLL's does. A tobiid older than this
 * library refuses stream 0 TOBII_ERROR_NOT_SUPPORTED, which the device
 * warns of once. Unlike the DLL, a subscribe whose round trip fails for
 * want of a connection keeps nothing (TOBII_ERROR_CONNECTION_FAILED), and
 * tobii_device_reconnect has tobiid hold stream 0's stream again, rather
 * than subscribe every entry anew. The prototypes took void const* for the
 * callback before; the ABI is the same. In C a callback of another function
 * type now gets an incompatible-pointer-types diagnostic (an error by
 * default since GCC 14), but one held in a void pointer still converts
 * silently, and it is now called: it must have the declared signature. In
 * C++ both are errors. */
typedef struct tobii_custom_stream_data_t
{
    uint32_t stream;
    uint32_t reserved_04;
    uint64_t size;
    void const* data;
} tobii_custom_stream_data_t;

typedef void ( *tobii_custom_stream_callback_t )( tobii_custom_stream_data_t const* data,
    void* user_data );

TOBII_API tobii_error_t TOBII_CALL tobii_custom_stream_subscribe( tobii_device_t* device,
    tobii_custom_stream_callback_t callback, uint32_t stream, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_custom_stream_unsubscribe( tobii_device_t* device,
    uint32_t stream );

/* Image collection, internal stream 6: the frames of the tracker's stream
 * 0x508 (image_collection in its catalogue), one record each. The layout is
 * the DLL's (0x5c0 bytes, its dispatch's record at 0x180155560..0x180155d8d);
 * the names are ours, after the tracker's own names for the stream's
 * columns: the frame's time on the tracker's clock and on
 * tobii_system_clock's, keys 2 to 7, the regions (key 8, "Region group Conf
 * list": the first region_count set, 10 at most), keys 9 to 13, the LEDs
 * (key 14, "LED Conf list": the first led_count set, 64 at most) and the
 * image (key 15: size bytes at data). A region's x and y and an LED's value
 * are 16.16 values the DLL turns into floats; what any field means is not
 * known.
 *
 * The ET5 refuses the stream, answering its start (command 1220) with
 * 0x20000505, most likely for want of an open realm (inferred), so the
 * subscribe answers TOBII_ERROR_INTERNAL there, as the DLL's does, and keeps
 * nothing. No frame of the stream was ever seen, so this library delivers
 * none: the callback is never called, and the layout comes from the DLL
 * alone, with nothing to check it against.
 *
 * The answers come as in the DLL: TOBII_ERROR_CALLBACK_IN_PROGRESS inside a
 * callback (checked first here), TOBII_ERROR_INVALID_PARAMETER for a null
 * device or callback, TOBII_ERROR_NOT_SUPPORTED when the tracker's catalogue
 * does not list 0x508 (before the rest, TOBII_ERROR_NOT_SUBSCRIBED too) and
 * TOBII_ERROR_ALREADY_SUBSCRIBED while image collection is subscribed. The
 * subscribe then has tobiid hold the stream for the connection, which the
 * tracker is to run (its command 1220), and answers what came of that as
 * the DLL maps the tracker's result: TOBII_ERROR_NO_ERROR when the tracker
 * took it, and when the tracker was lost meanwhile or did not answer in
 * time, the subscription kept for a reconnect to hold again, as the DLL
 * keeps it; TOBII_ERROR_NOT_SUPPORTED or TOBII_ERROR_OPERATION_FAILED for a
 * refusal as not supported or as too many subscribers; and
 * TOBII_ERROR_INTERNAL for any other, 0x20000505 among them. It waits ~65 s
 * at most for the start. The unsubscribe answers TOBII_ERROR_NO_ERROR once
 * the subscription is gone, whatever the stop (1230) came to, which it
 * waits ~55 s at most for. Either call may first wait ~12 s for the
 * catalogue, once per connection, and a fetch that fails answers as for the
 * custom stream (above). A tobiid older than this library refuses the
 * subscribe TOBII_ERROR_NOT_SUPPORTED, which the device warns of once.
 * Unlike the DLL, a subscribe whose round trip fails for want of a
 * connection keeps nothing (TOBII_ERROR_CONNECTION_FAILED), and
 * tobii_device_reconnect has tobiid hold the stream again rather than
 * subscribe it anew, so it does not fail when the tracker refuses the
 * stream. The prototype took void const* for the callback before; the ABI
 * is the same. In C a callback of another function type now gets an
 * incompatible-pointer-types diagnostic (an error by default since GCC 14),
 * but one held in a void pointer still converts silently. In C++ both are
 * errors. */
typedef struct tobii_image_collection_region_t
{
    uint32_t id;
    float x;
    float y;
    uint32_t words[ 6 ];
} tobii_image_collection_region_t;

typedef struct tobii_image_collection_led_t
{
    uint32_t words[ 3 ];
    float value;
} tobii_image_collection_led_t;

typedef struct tobii_image_collection_t
{
    int64_t timestamp_tracker_us;
    int64_t timestamp_system_us;
    uint32_t bits_per_pixel;
    uint32_t frame_format;
    uint32_t frame_number;
    uint32_t color_information;
    uint32_t encryption_type;
    uint32_t tdi_system_type;
    int region_count;
    tobii_image_collection_region_t regions[ 10 ];
    uint32_t little_endian;
    uint32_t x_skip;
    uint32_t y_skip;
    uint32_t x_bin;
    uint32_t y_bin;
    int led_count;
    tobii_image_collection_led_t leds[ 64 ];
    uint32_t reserved_5ac;
    uint64_t size;
    void const* data;
} tobii_image_collection_t;

typedef void ( *tobii_image_collection_callback_t )( tobii_image_collection_t const* image_collection,
    void* user_data );

TOBII_API tobii_error_t TOBII_CALL tobii_image_collection_subscribe( tobii_device_t* device,
    tobii_image_collection_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_image_collection_unsubscribe( tobii_device_t* device );

/* Internal stream ids (the names of 0..2 are inferred): 0 image, 1 clean IR,
 * 2 custom, 3 low-frequency head rotation, 4 low-frequency head position,
 * 5 multiple faces position, 6 image collection, 7 wearable limited image,
 * 8 secondary camera image. Supported: 0 (the IR image), and 2 (the custom
 * stream, above) and 6 (image collection, above) while the tracker's
 * catalogue lists their streams, 0x1770 and 0x508, as the ET5's does. That
 * is the DLL's answer for an ET5 when it drives the tracker itself: it asks
 * that tracker's catalogue about 0..2 and 6, and supports none of 3, 4, 5,
 * 7 and 8 for any tracker it drives itself (below). Image collection is
 * supported, as in the DLL, though the ET5 refuses to start it. This
 * library answers 0 and 1 by what it serves (the IR image, and no clean
 * IR), and 2 and 6 from the catalogue tobiid reports, asked for once per
 * connection (and again after a fetch that failed), which a call for
 * either may wait ~12 s for, after the calls other threads have under way
 * on the device or queued ahead of it. As in the DLL, the answer is never
 * an error once the arguments check out: a catalogue that cannot be had
 * reports 2 and 6 unsupported, with TOBII_ERROR_NO_ERROR. Behind the Tobii
 * service the DLL never supports 0 or 1 and answers 2..8 from the streams
 * the service lists, never captured for an ET5. An unknown id is reported
 * unsupported, not an error; an id above INT32_MAX is
 * TOBII_ERROR_INVALID_PARAMETER. */
TOBII_API tobii_error_t TOBII_CALL tobii_internal_stream_supported( tobii_device_t* device,
    uint32_t stream, tobii_supported_t* supported );

/* Internal streams 3, 4, 5, 7 and 8: TOBII_ERROR_CALLBACK_IN_PROGRESS from
 * inside a callback, then TOBII_ERROR_INVALID_PARAMETER for a null device or,
 * subscribing, a null callback (the DLL checks both before the callback),
 * and otherwise always TOBII_ERROR_NOT_SUPPORTED, subscribed or not, with no
 * request to the daemon. That is the DLL's answer for any tracker it drives
 * itself (a URL other than tobii-prp:// and tprp-tcp://): its support check
 * (0x180157860) has no case for these streams, and its in-process tracker
 * module (legacy TTP) refuses them too. Behind the Tobii service the DLL
 * answers from the streams the service lists, never captured for an ET5.
 * There it calls a low-frequency head callback with a pointer to a 24-byte
 * record (names ours):
 *
 *     struct { int64_t timestamp_us; tobii_validity_t validity; float xyz[ 3 ]; }
 *
 * where timestamp_us is the service's package timestamp, one validity covers
 * the three values, and xyz is copied from the service unchanged, so its
 * units, frame and rate are the service's and not known. No type is declared
 * for it, and the callbacks stay void const*: nothing is ever delivered. */
TOBII_API tobii_error_t TOBII_CALL tobii_low_frequency_head_rotation_subscribe( tobii_device_t* device, void const* callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_low_frequency_head_rotation_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_low_frequency_head_position_subscribe( tobii_device_t* device, void const* callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_low_frequency_head_position_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_multiple_faces_position_subscribe( tobii_device_t* device, void const* callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_multiple_faces_position_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_wearable_limited_image_subscribe( tobii_device_t* device, void const* callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_wearable_limited_image_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_secondary_camera_image_subscribe( tobii_device_t* device, void const* callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_secondary_camera_image_unsubscribe( tobii_device_t* device );

/* Internal capability ids (only the name of 0 is the DLL's): 0 eyeball
 * center, 1 diagnostic images, 2 remote wake, 3 power save, 4 face id,
 * 5 logs. Supported: 0 only, since tobii_gaze_data_t carries each eye's
 * eyeball centre. For 0 that is what the DLL would answer on its TTP path
 * (inferred); on its PRP path, behind the Tobii service, 0 is
 * TOBII_ERROR_NOT_SUPPORTED. The DLL answers 1..5 from device lists never
 * captured for an ET5. An unknown id is reported unsupported, not an error;
 * a negative id is TOBII_ERROR_INVALID_PARAMETER. */
TOBII_API tobii_error_t TOBII_CALL tobii_internal_capability_supported( tobii_device_t* device,
    int32_t capability, tobii_supported_t* supported );

/* One tracker/host clock pair: the tracker clock (the clock of gaze data's
 * timestamp_tracker_us) read tracker_us at some host time between
 * system_start_us and system_end_us, all in microseconds. The host clock is
 * tobii_system_clock's: CLOCK_MONOTONIC, not the DLL's
 * QueryPerformanceCounter, though like it monotonic with an undefined epoch.
 * The pair comes from the first gaze frame the daemon receives after the call
 * (the tracker is started if needed), and the bracket is a fixed 30 ms
 * before the daemon read it. The DLL times a round trip instead; its own
 * offset estimator skips pairs wider than 6 ms, though it still returns them
 * with TOBII_ERROR_NO_ERROR. The callbacks' timestamp_us (and gaze data's
 * timestamp_system_us) need no pair: the daemon has already put them on the
 * host clock (tobii_streams.h); gaze data's timestamp_tracker_us is the
 * tracker time a pair maps. The tracker's clock may restart when the tracker
 * re-initialises, so a pair holds until then only. Nothing is written unless
 * the call succeeds; TOBII_ERROR_NOT_AVAILABLE while the tracker is paused,
 * TOBII_ERROR_CONNECTION_FAILED when no tracker is plugged in. Layout from
 * the DLL (24 bytes); the field names follow the older public headers. */
typedef struct tobii_timesync_data_t
{
    int64_t system_start_us;
    int64_t system_end_us;
    int64_t tracker_us;
} tobii_timesync_data_t;

TOBII_API tobii_error_t TOBII_CALL tobii_timesync( tobii_device_t* device,
    tobii_timesync_data_t* timesync );

/* One entry of the tracker's stream catalogue. `type` is the Stream Engine's
 * stream type the DLL maps the tracker's stream id to (0x500 gaze -> 1,
 * 0x501 image -> 2, 0x502 -> 3, 0x503 -> 14, 0x504 presence -> 4, 0x505 -> 5,
 * 0x506 -> 8, 0x507 -> 9, 0x508 image_collection -> 11, 0x50a -> 6,
 * 0x1770 algodbg -> 7, anything else -> 0). `value` is the tracker's number
 * for the stream (1000 for image_collection on the ET5, else 0) and `text` a
 * second string (empty on the ET5); what they mean is unknown. Offsets from
 * the DLL; the size (136 bytes) is inferred and the field names are ours. */
typedef struct tobii_stream_type_t
{
    int type;
    uint32_t value;
    char name[ 64 ];
    char text[ 64 ];
} tobii_stream_type_t;

typedef void ( *tobii_stream_type_receiver_t )( tobii_stream_type_t const* stream_type,
    void* user_data );

/* The tracker's stream catalogue, one receiver call per stream in the
 * tracker's order (nine on the ET5). It is the catalogue the tracker reported
 * at its last init, where the DLL asks the tracker on every call. It lists
 * streams this library does not deliver, such as image_collection, which
 * the ET5 refuses to start. The DLL answers only on its TTP path and with
 * the internal feature group; this library has no licence gate.
 * TOBII_ERROR_NOT_SUPPORTED if the tracker reported no catalogue (the DLL
 * answers TOBII_ERROR_NO_ERROR with no calls) or the daemon is older than
 * this library; TOBII_ERROR_TIMED_OUT if no tracker has been seen. The
 * receiver may call back into the library; each entry is valid during its
 * call only. */
TOBII_API tobii_error_t TOBII_CALL tobii_enumerate_stream_types( tobii_device_t* device,
    tobii_stream_type_receiver_t receiver, void* user_data );

/* One column of one of the tracker's streams, as the tracker lists it
 * answering command 1210: the key the stream's frames carry it under, two
 * numbers (0 and 1 in every column on the ET5), the column's name ("Left User
 * 3D Position Guide" for gaze's key 0x25) and a second string (empty on the
 * ET5); what the numbers and the second string mean is unknown. The layout is
 * the DLL's (140 bytes, its receiver trampoline at 0x180001eb0); the names
 * are ours. */
typedef struct tobii_stream_type_column_t
{
    uint32_t key;
    uint32_t param_a;
    uint32_t param_b;
    char name[ 64 ];
    char text[ 64 ];
} tobii_stream_type_column_t;

typedef void ( *tobii_stream_type_column_receiver_t )( tobii_stream_type_column_t const* column,
    void* user_data );

/* The columns of one of the tracker's streams, one receiver call per column
 * in the tracker's order (39 for gaze on the ET5). stream_type is a Stream
 * Engine stream type, as tobii_enumerate_stream_types hands them out, which
 * the DLL maps to the tracker's stream id (1 -> 0x500, 2 -> 0x501,
 * 3 -> 0x502, 4 -> 0x504, 5 -> 0x505, 6 -> 0x50a, 7 and 12 -> 0x1770,
 * 8 -> 0x506, 9 -> 0x507, 11 -> 0x508, anything else -> 0); 10 is
 * TOBII_ERROR_NOT_SUPPORTED with nothing asked, as in the DLL. The columns
 * are those the tracker listed answering the command 1210 of the daemon's
 * last init that got an answer for the stream (one that lost it keeps the
 * one before), where the DLL asks the tracker on every call. That init asks
 * only about the streams the tracker's catalogue lists, so a stream it does
 * not list or whose 1210 it refused, and a type that maps to 0, are
 * TOBII_ERROR_NOT_SUPPORTED, where the DLL asks all the same and answers as
 * the tracker does; so is an answer with a bad entry, which the DLL answers
 * TOBII_ERROR_CONNECTION_FAILED. More than 73 columns is
 * TOBII_ERROR_CONNECTION_FAILED with no call, as in the DLL. TOBII_ERROR_NOT_SUPPORTED too from a daemon
 * older than this library; TOBII_ERROR_TIMED_OUT if no tracker has been seen.
 * The DLL answers only on its TTP path and with the internal feature group;
 * this library has no licence gate. The receiver may call back into the
 * library; each column is valid during its call only. The prototype took
 * void const* for the receiver before; the ABI is the same. In C a receiver
 * of another function type now gets an incompatible-pointer-types diagnostic
 * (an error by default since GCC 14), but one held in a void pointer still
 * converts silently, and it is now called: it must have the declared
 * signature. In C++ both are errors. */
TOBII_API tobii_error_t TOBII_CALL tobii_enumerate_stream_type_columns( tobii_device_t* device,
    uint32_t stream_type, tobii_stream_type_column_receiver_t receiver, void* user_data );

/* Pause the tracker (it sends no data until resumed), or resume it. The pause
 * is one state for the tracker, shared by every client as in the DLL: the
 * last call wins and any client may resume. Unlike the DLL, a pause or resume
 * the tracker accepts shows at once in TOBII_STATE_DEVICE_PAUSED and a
 * TOBII_NOTIFICATION_TYPE_DEVICE_PAUSED_STATE_CHANGED notification. A pause
 * ends when the client that paused last disconnects and whenever the tracker
 * re-initialises. Pausing during a calibration session is
 * TOBII_ERROR_CALIBRATION_BUSY, and no session starts while paused
 * (TOBII_ERROR_NOT_AVAILABLE). A resume the tracker does not answer still
 * succeeds: the daemon re-opens a tracker that stays silent, which resumes
 * it. */
TOBII_API tobii_error_t TOBII_CALL tobii_pause_device( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_resume_device( tobii_device_t* device );

/* PROVISIONAL. The tracker's hardware configuration (its command 2120). The
 * layout is the DLL's (2472 bytes; offsets from its copy at 0x18014abe0 and
 * its PRP deserialiser at 0x180045056, which load the 64-bit fields as
 * double); the field names are ours, and what the fields mean is not known.
 * The values come from the one answer ever captured (Windows): 16.16 values
 * unscaled, 32.32 values scaled as lengths, in mm. On Linux the ET5 has so
 * far answered 2120 with no data, so the call returns
 * TOBII_ERROR_NOT_SUPPORTED there. When it succeeds the whole struct is
 * written, zero past each count (the DLL leaves those slots untouched), and
 * a mode outside 0..2 is 0, as in the DLL. Nothing is written on an error. */
typedef struct tobii_hardware_configuration_entry_t
{
    int id;
    float param_a;
    float param_b;
    double position_xyz[ 3 ];
    double values[ 15 ];
    int width;
    int height;
    int param_c;
    int coefficient_count;
    double coefficients[ 64 ];
    double point_a_xyz[ 3 ];
    double point_b_xyz[ 3 ];
    double param_d;
} tobii_hardware_configuration_entry_t;

typedef struct tobii_hardware_configuration_t
{
    int entry_count;
    tobii_hardware_configuration_entry_t entries[ 2 ];
    int point_count;
    double points_xyz[ 40 ][ 3 ];
    int mode;
} tobii_hardware_configuration_t;

TOBII_API tobii_error_t TOBII_CALL tobii_hardware_configuration_get( tobii_device_t* device,
    tobii_hardware_configuration_t* configuration );

/* Properties 3 and 4 of the tracker's own property list (its command 1330),
 * 128 bytes each, NUL-terminated and zero after the NUL: the platform type
 * (property 3, which the DLL checks a licence's platform types against;
 * "IS5LEYETRACKER5" on the ET5) and property 4, which nothing names (empty on
 * the ET5, which does not send it). The layout is the DLL's (its strncpys at
 * 0x180150020 and 0x18015003d); the names are ours. A property of 120 bytes or
 * more comes cut to 119, as the DLL's tracker module keeps it. As the DLL:
 * TOBII_ERROR_INVALID_PARAMETER for a null device or info and
 * TOBII_ERROR_CALLBACK_IN_PROGRESS inside a callback (checked first here); the
 * DLL's TOBII_ERROR_INSUFFICIENT_LICENSE below the internal feature group is
 * not modelled (libtobii gates nothing on a licence). The strings are the ones
 * the tracker reported at its last init; TOBII_ERROR_NOT_SUPPORTED from a
 * tobiid that does not send them. Nothing is written on an error. */
typedef struct tobii_device_info_internal_t
{
    char platform_type[ 128 ];
    char property_4[ 128 ];
} tobii_device_info_internal_t;

TOBII_API tobii_error_t TOBII_CALL tobii_get_device_info_internal( tobii_device_t* device,
    tobii_device_info_internal_t* info );

/* The calibration's stimulus points as the DLL lays them out: a count, then
 * at most 32 records of 36 bytes (1156 bytes; its copier at 0x180001800 and
 * its fixed 0x488-byte PRP body, a size word and then this, at 0x180042517
 * and 0x18004563e). The DLL copies each record as nine 32-bit words and never
 * reads one, so what they hold is not known; the names are ours. The
 * prototype took void* before; the ABI is the same. In C a pointer to another
 * type now gets an incompatible-pointer-types diagnostic (an error by default
 * since GCC 14), while a void pointer still converts silently; in C++ both
 * are errors. */
typedef struct tobii_calibration_stimulus_point_t
{
    uint32_t words[ 9 ];
} tobii_calibration_stimulus_point_t;

typedef struct tobii_calibration_stimulus_points_t
{
    int point_count;
    tobii_calibration_stimulus_point_t points[ 32 ];
} tobii_calibration_stimulus_points_t;

/* TOBII_ERROR_CALLBACK_IN_PROGRESS from inside a callback, then
 * TOBII_ERROR_INVALID_PARAMETER for a null device or points (the DLL checks
 * both before the callback), and otherwise always TOBII_ERROR_NOT_SUPPORTED,
 * with nothing written and no request to the daemon. That is the answer of
 * the DLL's in-process tracker module (legacy TTP) for an ET5, whose property
 * list never holds the stimulus points (PRP property 0x13); behind Tobii's
 * service (tobii-prp://) the answer was never captured. */
TOBII_API tobii_error_t TOBII_CALL tobii_calibration_stimulus_points_get( tobii_device_t* device,
    tobii_calibration_stimulus_points_t* points );

/* Face id, as the DLL lays it out; the names are ours, and what the values
 * mean is not known. The state is PRP property 0xd: a state, 0..3 (the DLL
 * maps the service's 1..4 onto it and anything else to 0, 0x180001710), and
 * a second word copied as it comes. The parameters are property 0xe, one
 * word (0x1800014f0). An enrolment takes one word identifying the face, and
 * gives one result word, 0..3, or 4 for anything else (0x1800016c0). The
 * callbacks get the record and the user data (0x1801540af, 0x180154023).
 *
 * Every call answers TOBII_ERROR_CALLBACK_IN_PROGRESS from inside a
 * callback, then TOBII_ERROR_INVALID_PARAMETER for a null device, face id,
 * result, callback, parameters or state (the DLL checks those before the
 * callback), and otherwise always TOBII_ERROR_NOT_SUPPORTED, never
 * TOBII_ERROR_NOT_SUBSCRIBED, with nothing read or written and no request to
 * the daemon. That is the DLL's answer for a tracker it drives itself (legacy
 * TTP), whose module lists neither property and refuses both enrol actions;
 * behind Tobii's service the answer was never captured. These prototypes took
 * void pointers before; the ABI is the same. In C a pointer to another type,
 * or a callback of another function type, now gets an incompatible-pointer-
 * types diagnostic (an error by default since GCC 14), while a void pointer
 * still converts silently; in C++ both are errors. */
typedef struct tobii_face_id_state_t
{
    uint32_t state;
    uint32_t value;
} tobii_face_id_state_t;

typedef struct tobii_face_id_parameters_t
{
    uint32_t value;
} tobii_face_id_parameters_t;

typedef void ( *tobii_face_id_state_callback_t )( tobii_face_id_state_t const* state, void* user_data );
typedef void ( *tobii_face_id_parameters_callback_t )( tobii_face_id_parameters_t const* parameters,
    void* user_data );

TOBII_API tobii_error_t TOBII_CALL tobii_face_id_enroll( tobii_device_t* device, uint32_t const* face_id,
    uint32_t* result );
TOBII_API tobii_error_t TOBII_CALL tobii_face_id_enroll_clear( tobii_device_t* device, uint32_t const* face_id );
TOBII_API tobii_error_t TOBII_CALL tobii_face_id_state_subscribe( tobii_device_t* device,
    tobii_face_id_state_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_face_id_state_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_face_id_parameters_subscribe( tobii_device_t* device,
    tobii_face_id_parameters_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_face_id_parameters_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_get_face_id_state( tobii_device_t* device, tobii_face_id_state_t* state );
TOBII_API tobii_error_t TOBII_CALL tobii_get_face_id_parameters( tobii_device_t* device,
    tobii_face_id_parameters_t* parameters );
TOBII_API tobii_error_t TOBII_CALL tobii_set_face_id_parameters( tobii_device_t* device,
    tobii_face_id_parameters_t const* parameters );

/* Usage statistics for Tobii: two strings of at most 255 bytes each (what they
 * hold is not known). TOBII_ERROR_CALLBACK_IN_PROGRESS from inside a
 * callback, then TOBII_ERROR_INVALID_PARAMETER for a null device or string
 * (the DLL checks those before the callback) or a longer string, and
 * otherwise TOBII_ERROR_NOT_AVAILABLE, with nothing sent: the DLL's answer
 * for a tracker it drives itself (legacy TTP) when Tobii's Stream Engine
 * Services do not run, as on Linux. Behind Tobii's service it answers
 * TOBII_ERROR_NOT_SUPPORTED. The prototype took (void const* data, size_t
 * size) before, which is not what the DLL reads. In C a size passed as b now
 * gets an int-conversion diagnostic (an error by default since GCC 14), unless
 * it is a literal 0, a null pointer, while the void pointer data still
 * converts silently; in C++ the void pointer is an error. */
TOBII_API tobii_error_t TOBII_CALL tobii_send_statistics( tobii_device_t* device, char const* a, char const* b );
/* TOBII_ERROR_INVALID_PARAMETER for a null device or display_info, and
 * otherwise always TOBII_ERROR_NOT_SUPPORTED, display_info unread, as the DLL
 * answers on every path. It checks for no callback, so a call from inside
 * one gets the same answers. The DLL's getter fills 72 bytes, a 64-byte string
 * and two ints, whose meaning is not known. */
TOBII_API tobii_error_t TOBII_CALL tobii_set_display_info( tobii_device_t* device, void const* display_info );
/* Always TOBII_ERROR_NOT_SUPPORTED, whatever the arguments, a null device and
 * a call from inside a callback included, as in the DLL, which checks and reads
 * nothing (it treats its first argument as an API instance when it logs the
 * refusal, so it may take a tobii_api_t* there; inferred). */
TOBII_API tobii_error_t TOBII_CALL tobii_set_fw_upgrade_allowed( tobii_device_t* device, uint32_t allowed );

/* Everything below: NOT IMPLEMENTED, returns TOBII_ERROR_NOT_SUPPORTED. */

TOBII_API tobii_error_t TOBII_CALL tobii_clean_ir_subscribe( tobii_device_t* device, void const* callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_clean_ir_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_diagnostic_images_retrieve( tobii_device_t* device, void const* receiver, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_diagnostics_dump_images( tobii_device_t* device, uint32_t a, uint32_t b );
TOBII_API tobii_error_t TOBII_CALL tobii_diagnostics_get_data( tobii_device_t* device, uint32_t kind, void const* receiver, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_diagnostics_image_subscribe( tobii_device_t* device, void const* callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_diagnostics_image_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_display_id_subscribe( tobii_device_t* device, void const* callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_display_id_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_enable_extension( tobii_device_t* device, uint32_t extension );
TOBII_API tobii_error_t TOBII_CALL tobii_enumerate_enabled_extensions( tobii_device_t* device, void const* receiver, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_enumerate_extensions( tobii_device_t* device, void const* receiver, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_enumerate_illumination_modes( tobii_device_t* device, void const* receiver, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_get_combined_gaze_hid_track_box( tobii_device_t* device, void* track_box );
TOBII_API tobii_error_t TOBII_CALL tobii_get_configuration_key( tobii_device_t* device, void* key, void* value );
TOBII_API tobii_error_t TOBII_CALL tobii_get_display_id( tobii_device_t* device, void* display_id );
TOBII_API tobii_error_t TOBII_CALL tobii_get_display_info( tobii_device_t* device, void* display_info );
TOBII_API tobii_error_t TOBII_CALL tobii_get_gaze_hid_enabled( tobii_device_t* device, void* enabled );
TOBII_API tobii_error_t TOBII_CALL tobii_get_illumination_mode( tobii_device_t* device, void* mode );
TOBII_API tobii_error_t TOBII_CALL tobii_logs_retrieve( tobii_device_t* device, void const* receiver, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_open_realm( tobii_device_t* device, uint32_t realm, void const* key, uint32_t key_size );
TOBII_API tobii_error_t TOBII_CALL tobii_power_save_activate( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_power_save_deactivate( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_remote_wake_activate( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_remote_wake_deactivate( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_send_custom_command( tobii_device_t* device, uint32_t command, void const* data, size_t size, void const* receiver, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_set_display_id( tobii_device_t* device, uint32_t display_id );
TOBII_API tobii_error_t TOBII_CALL tobii_set_illumination_mode( tobii_device_t* device, void const* mode );

#ifdef __cplusplus
}
#endif

#endif /* TOBII_INTERNAL_H */
