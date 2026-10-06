/* tobii/tobii_advanced.h — per-eye gaze data, as provided by libtobii.so.
 * Companion to tobii/tobii.h and tobii/tobii_streams.h.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef TOBII_ADVANCED_H
#define TOBII_ADVANCED_H

#include "tobii.h"
#include "tobii_streams.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Tracker frame, mm (origin at the tracker, z towards the user). The pupil
 * diameter, in mm, is the tracker's own, as the DLL hands it out: valid
 * while the eye's status is below 2, as in the DLL (the ET5 sends 0 for a
 * tracked eye, 4 for a lost one), and passed on when it is not. */
typedef struct tobii_gaze_data_eye_t
{
    tobii_validity_t gaze_origin_validity;
    float gaze_origin_from_eye_tracker_mm_xyz[ 3 ];
    float gaze_origin_in_track_box_normalized_xyz[ 3 ];

    tobii_validity_t gaze_point_validity;
    float gaze_point_from_eye_tracker_mm_xyz[ 3 ];
    float gaze_point_on_display_normalized_xy[ 2 ];

    tobii_validity_t eyeball_center_validity;
    float eyeball_center_from_eye_tracker_mm_xyz[ 3 ];

    tobii_validity_t pupil_validity;
    float pupil_diameter_mm;
} tobii_gaze_data_eye_t;

/* timestamp_tracker_us is on the tracker's clock, which may restart when the
 * tracker re-initialises; timestamp_system_us is the same instant on the
 * tobii_system_clock clock, as the DLL gives it, not the time the sample was
 * received: the timestamp_us the frame's gaze point carries
 * (tobii_streams.h). */
typedef struct tobii_gaze_data_t
{
    int64_t timestamp_tracker_us;
    int64_t timestamp_system_us;
    tobii_gaze_data_eye_t left;
    tobii_gaze_data_eye_t right;
} tobii_gaze_data_t;

typedef void ( *tobii_gaze_data_callback_t )( tobii_gaze_data_t const* gaze_data, void* user_data );

TOBII_API tobii_error_t TOBII_CALL tobii_gaze_data_subscribe( tobii_device_t* device,
    tobii_gaze_data_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_gaze_data_unsubscribe( tobii_device_t* device );

typedef void ( *tobii_digital_syncport_callback_t )( uint32_t signal, int64_t timestamp_tracker_us,
    int64_t timestamp_system_us, void* user_data );

/* The ET5 has no digital sync port. TOBII_ERROR_CALLBACK_IN_PROGRESS from
 * inside a callback, then TOBII_ERROR_INVALID_PARAMETER for a null device
 * (the DLL checks it before the callback), and otherwise always
 * TOBII_ERROR_NOT_SUPPORTED, whatever the callback (the DLL never checks it
 * for null), never TOBII_ERROR_NOT_SUBSCRIBED, with no request to the daemon:
 * the DLL's answer for an ET5 it drives itself (legacy TTP), before any
 * licence check; behind Tobii's service it is NOT_SUPPORTED too. */
TOBII_API tobii_error_t TOBII_CALL tobii_digital_syncport_subscribe( tobii_device_t* device,
    tobii_digital_syncport_callback_t callback, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_digital_syncport_unsubscribe( tobii_device_t* device );

typedef char tobii_face_type_t[ 64 ];
typedef void ( *tobii_face_type_receiver_t )( tobii_face_type_t const face_type, void* user_data );

/* TOBII_ERROR_CALLBACK_IN_PROGRESS from inside a callback, then
 * TOBII_ERROR_INVALID_PARAMETER for a null device, receiver or face type
 * (the DLL checks those before the callback), and otherwise always
 * TOBII_ERROR_NOT_SUPPORTED, with the receiver never called, nothing written
 * and no request to the daemon. Inferred, not the DLL's known answer: for an
 * ET5 it drives itself (legacy TTP), the DLL asks the tracker for the face
 * types (command 3320) and the face type (3300), whose answers were never
 * captured, and refuses to set one (3310) below the config feature group
 * with TOBII_ERROR_INSUFFICIENT_LICENSE; libtobii gates nothing on a
 * licence. The ET5 lists no face types among its capabilities (1350), so
 * TOBII_CAPABILITY_FACE_TYPE is not supported. Behind Tobii's service the
 * DLL answers NOT_SUPPORTED. */
TOBII_API tobii_error_t TOBII_CALL tobii_enumerate_face_types( tobii_device_t* device,
    tobii_face_type_receiver_t receiver, void* user_data );
TOBII_API tobii_error_t TOBII_CALL tobii_set_face_type( tobii_device_t* device,
    tobii_face_type_t const face_type );
TOBII_API tobii_error_t TOBII_CALL tobii_get_face_type( tobii_device_t* device,
    tobii_face_type_t* face_type );

#ifdef __cplusplus
}
#endif

#endif /* TOBII_ADVANCED_H */
