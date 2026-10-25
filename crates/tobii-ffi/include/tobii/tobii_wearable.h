/* tobii/tobii_wearable.h — head-mounted devices. Declared so wearable-aware
 * code builds; nothing here applies to the screen-based ET5.
 * tobii_lens_configuration_writable answers
 * TOBII_LENS_CONFIGURATION_NOT_WRITABLE. Every other entry point answers
 * TOBII_ERROR_CALLBACK_IN_PROGRESS from inside a callback, then
 * TOBII_ERROR_INVALID_PARAMETER for a null device, callback or lens
 * configuration (the DLL checks those before the callback), and otherwise
 * always TOBII_ERROR_NOT_SUPPORTED, never TOBII_ERROR_NOT_SUBSCRIBED, with
 * nothing written and no request to the daemon. That is the DLL's answer for
 * an ET5 it drives itself (legacy TTP): it offers the wearable streams only
 * to a tracker whose stream catalogue holds stream 0x505, and the lens
 * configuration only to one whose generation starts with "VR"; behind
 * Tobii's service (tobii-prp://) the answer was never captured. Companion to
 * tobii/tobii.h.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef TOBII_WEARABLE_H
#define TOBII_WEARABLE_H

#include "tobii.h"
#include "tobii_streams.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct tobii_wearable_eye_t
{
    tobii_validity_t pupil_position_in_sensor_area_validity;
    float pupil_position_in_sensor_area_xy[ 2 ];
    tobii_validity_t position_guide_validity;
    float position_guide_xy[ 2 ];
    tobii_validity_t blink_validity;
    tobii_state_bool_t blink;
} tobii_wearable_eye_t;

typedef struct tobii_wearable_consumer_data_t
{
    int64_t timestamp_us;
    tobii_wearable_eye_t left;
    tobii_wearable_eye_t right;
    tobii_validity_t gaze_origin_combined_validity;
    float gaze_origin_combined_mm_xyz[ 3 ];
    tobii_validity_t gaze_direction_combined_validity;
    float gaze_direction_combined_normalized_xyz[ 3 ];
    tobii_validity_t convergence_distance_validity;
    float convergence_distance_mm;
    tobii_state_bool_t improve_user_position_hmd;
    tobii_state_bool_t increase_eye_relief;
} tobii_wearable_consumer_data_t;

typedef void ( *tobii_wearable_consumer_data_callback_t )(
    tobii_wearable_consumer_data_t const* data, void* user_data );

/* One eye of tobii_wearable_advanced_data_t: the six fields the 4.1 docs list
 * for it, each a validity and a value, in their order, 72 bytes, as both DLLs
 * lay them out, with the eyes at +0x10 and +0x58 (4.1's dispatch,
 * 0x180156360..0x180156592, fills a 208-byte record; 4.25's,
 * 0x18008e4e0..0x18008e739, a 216-byte one — see the record below). The eye
 * itself did not change: 4.25's loop writes the same eighteen dwords over
 * 0x00..0x47. The type's name is ours; this
 * header used the 32-byte consumer eye, tobii_wearable_eye_t, there before,
 * which made the record 128 bytes, so code built against that layout read the
 * wrong offsets (no callback ever runs with libtobii: the stream is not
 * supported). */
typedef struct tobii_wearable_advanced_eye_t
{
    tobii_validity_t gaze_origin_validity;
    float gaze_origin_mm_xyz[ 3 ];
    tobii_validity_t gaze_direction_validity;
    float gaze_direction_normalized_xyz[ 3 ];
    tobii_validity_t pupil_diameter_validity;
    float pupil_diameter_mm;
    tobii_validity_t pupil_position_in_sensor_area_validity;
    float pupil_position_in_sensor_area_xy[ 2 ];
    tobii_validity_t position_guide_validity;
    float position_guide_xy[ 2 ];
    tobii_validity_t blink_validity;
    tobii_state_bool_t blink;
} tobii_wearable_advanced_eye_t;

/* 216 bytes, which is 4.25's record and not 4.1's 208: 4.25 inserts a
 * validity and a float before the two trailing flags (see unknown_c8_validity
 * below). This is the PRP compound stream WEARABLE_ADVANCED, enum 4 of five,
 * delivered out of the callback slot dev+0x293b0 by either of two builders —
 * 0x18008e4e0..0x18008e739 for the advanced package, 0x18008e740..0x18008e999
 * for the legacy combined WEARABLE one. The first field takes the package's
 * own clock and the second adds the device's offset (0x18008e51a and
 * 0x18008e522), which is what fixes their order. */
typedef struct tobii_wearable_advanced_data_t
{
    int64_t timestamp_tracker_us;
    int64_t timestamp_system_us;
    tobii_wearable_advanced_eye_t left;
    tobii_wearable_advanced_eye_t right;
    tobii_validity_t gaze_origin_combined_validity;
    float gaze_origin_combined_mm_xyz[ 3 ];
    tobii_validity_t gaze_direction_combined_validity;
    float gaze_direction_combined_normalized_xyz[ 3 ];
    tobii_validity_t convergence_distance_validity;
    float convergence_distance_mm;
    /* The pair 4.25 inserts and 4.1 has not, from the package's +0xb8 (a
     * byte, through cmpb/setne) and +0xbc (0x18008e697 and 0x18008e6a3). It
     * is what moves the two flags below from +0xc8/+0xcc to +0xd0/+0xd4 and
     * grows the record by 8. What it means is not recoverable from the
     * binary: neither builder names anything about itself, each carrying one
     * rip-relative operand and that the /GS cookie, so these names are
     * positional, as tobii_gaze_raw_t's key_* fields are. The legacy
     * combined WEARABLE package never writes the pair — its builder has no
     * store to either offset — and the record is memset before filling
     * (0x18008e50a, 0xc8 bytes from +0x10), so a subscriber on that path
     * reads zero here. Written unconditionally on the advanced path: the
     * block is straight-line, with no version gate. The wire is gated
     * (0x1801fc168) and appends rather than inserts, so it stays
     * backward-compatible where this struct does not. */
    tobii_validity_t unknown_c8_validity;
    float unknown_cc;
    tobii_state_bool_t improve_user_position_hmd;
    tobii_state_bool_t increase_eye_relief;
} tobii_wearable_advanced_data_t;

typedef void ( *tobii_wearable_advanced_data_callback_t )(
    tobii_wearable_advanced_data_t const* data, void* user_data );

typedef struct tobii_lens_configuration_t
{
    float left_xyz[ 3 ];
    float right_xyz[ 3 ];
} tobii_lens_configuration_t;

typedef enum tobii_lens_configuration_writable_t
{
    TOBII_LENS_CONFIGURATION_NOT_WRITABLE,
    TOBII_LENS_CONFIGURATION_WRITABLE,
} tobii_lens_configuration_writable_t;

/* The extended lens configuration Stream Engine 4.25 adds beside the one
 * above: PRP property 0x17 instead of 0xa, and a 48-byte value of four
 * float[3] at 0x00/0x0c/0x18/0x24 — a per-eye offset and rotation, where the
 * first two vectors sit at the basic struct's offsets. The basic getter does
 * not forward to it. Names of the members are ours; the DLL's thunk moves
 * twelve dwords and names nothing. No device advertises the property, so
 * libtobii.so answers TOBII_ERROR_NOT_SUPPORTED, as it does for the basic
 * one. */
typedef struct tobii_lens_configuration_extended_per_eye_t
{
    float offset_mm_xyz[ 3 ];
    float rotation_xyz[ 3 ];
} tobii_lens_configuration_extended_per_eye_t;

typedef struct tobii_lens_configuration_extended_t
{
    tobii_lens_configuration_extended_per_eye_t left;
    tobii_lens_configuration_extended_per_eye_t right;
} tobii_lens_configuration_extended_t;

typedef tobii_lens_configuration_writable_t tobii_lens_configuration_extended_writable_t;

TOBII_API tobii_error_t TOBII_CALL tobii_get_lens_configuration_extended( tobii_device_t* device,
    tobii_lens_configuration_extended_t* lens_config );
TOBII_API tobii_error_t TOBII_CALL tobii_set_lens_configuration_extended( tobii_device_t* device,
    tobii_lens_configuration_extended_t const* lens_config );
TOBII_API tobii_error_t TOBII_CALL tobii_lens_configuration_extended_writable(
    tobii_device_t* device, tobii_lens_configuration_extended_writable_t* writable );

typedef enum tobii_wearable_foveated_tracking_state_t
{
    TOBII_WEARABLE_FOVEATED_TRACKING_STATE_TRACKING,
    TOBII_WEARABLE_FOVEATED_TRACKING_STATE_EXTRAPOLATED,
    TOBII_WEARABLE_FOVEATED_TRACKING_STATE_LAST_KNOWN,
} tobii_wearable_foveated_tracking_state_t;

typedef struct tobii_wearable_foveated_gaze_t
{
    int64_t timestamp_us;
    tobii_wearable_foveated_tracking_state_t tracking_state;
    float gaze_direction_combined_normalized_xyz[ 3 ];
} tobii_wearable_foveated_gaze_t;

typedef void ( *tobii_wearable_foveated_gaze_callback_t )(
    tobii_wearable_foveated_gaze_t const* data, void* user_data );

TOBII_API tobii_error_t TOBII_CALL tobii_wearable_consumer_data_subscribe( tobii_device_t* device,
    tobii_wearable_consumer_data_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_wearable_consumer_data_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_wearable_advanced_data_subscribe( tobii_device_t* device,
    tobii_wearable_advanced_data_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_wearable_advanced_data_unsubscribe( tobii_device_t* device );
TOBII_API tobii_error_t TOBII_CALL tobii_get_lens_configuration( tobii_device_t* device,
    tobii_lens_configuration_t* lens_config );
TOBII_API tobii_error_t TOBII_CALL tobii_set_lens_configuration( tobii_device_t* device,
    tobii_lens_configuration_t const* lens_config );
TOBII_API tobii_error_t TOBII_CALL tobii_lens_configuration_writable( tobii_device_t* device,
    tobii_lens_configuration_writable_t* writable );
TOBII_API tobii_error_t TOBII_CALL tobii_wearable_foveated_gaze_subscribe( tobii_device_t* device,
    tobii_wearable_foveated_gaze_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_wearable_foveated_gaze_unsubscribe( tobii_device_t* device );

#ifdef __cplusplus
}
#endif

#endif /* TOBII_WEARABLE_H */
