/* Proves that a Stream-Engine-shaped C program compiles against the tobii
 * headers, links against libtobii.so and gets the answers they promise.
 * Needs neither the tobiid daemon nor a tracker: every call here is answered
 * locally or rejected before any device work would happen.
 *
 * Built and run by `make verify-abi`, which generates abi-symbols.inc (one
 * X(name) per exported symbol) from crates/tobii-ffi/abi-symbols.txt.
 *
 * SPDX-License-Identifier: MIT
 */

/* clock_gettime is POSIX, not C11. */
#define _POSIX_C_SOURCE 200809L

#include <tobii/tobii.h>
#include <tobii/tobii_advanced.h>
#include <tobii/tobii_config.h>
#include <tobii/tobii_internal.h>
#include <tobii/tobii_licensing.h>
#include <tobii/tobii_streams.h>
#include <tobii/tobii_wearable.h>

#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

/* Every listed symbol, by address: each must be declared (with C linkage) by
 * some header, or this does not compile, and exported, or it does not link. */
typedef void ( *any_fn_t )( void );
#define X( name ) ( any_fn_t ) & name,
static any_fn_t const abi_symbols[] = {
#include "abi-symbols.inc"
};
#undef X

static int url_count = 0;
static char first_url[ 256 ] = { 0 };

static void url_receiver( char const* url, void* user_data )
{
    assert( user_data == &url_count );
    assert( url != NULL );
    if( url_count++ == 0 && strlen( url ) < sizeof( first_url ) )
        strcpy( first_url, url );
}

static void head_pose_callback( tobii_head_pose_t const* head_pose, void* user_data )
{
    (void)head_pose;
    (void)user_data;
}

/* Callbacks of the entry points refused once their arguments check out:
 * typed as the headers declare them, and never called. */
static void wearable_consumer_callback( tobii_wearable_consumer_data_t const* data, void* user_data )
{
    (void)data;
    (void)user_data;
    assert( 0 );
}

static void wearable_advanced_callback( tobii_wearable_advanced_data_t const* data, void* user_data )
{
    (void)data;
    (void)user_data;
    assert( 0 );
}

static void wearable_foveated_callback( tobii_wearable_foveated_gaze_t const* data, void* user_data )
{
    (void)data;
    (void)user_data;
    assert( 0 );
}

static void syncport_callback( uint32_t signal, int64_t timestamp_tracker_us, int64_t timestamp_system_us,
    void* user_data )
{
    (void)signal;
    (void)timestamp_tracker_us;
    (void)timestamp_system_us;
    (void)user_data;
    assert( 0 );
}

static void face_type_receiver( tobii_face_type_t const face_type, void* user_data )
{
    (void)face_type;
    (void)user_data;
    assert( 0 );
}

static void foveated_callback( tobii_gaze_point_t const* gaze_point, void* user_data )
{
    (void)gaze_point;
    (void)user_data;
    assert( 0 );
}

static void column_receiver( tobii_stream_type_column_t const* column, void* user_data )
{
    (void)column;
    (void)user_data;
    assert( 0 );
}

static void custom_stream_callback( tobii_custom_stream_data_t const* data, void* user_data )
{
    (void)data;
    (void)user_data;
    assert( 0 );
}

static void face_id_state_callback( tobii_face_id_state_t const* state, void* user_data )
{
    (void)state;
    (void)user_data;
    assert( 0 );
}

static void face_id_parameters_callback( tobii_face_id_parameters_t const* parameters, void* user_data )
{
    (void)parameters;
    (void)user_data;
    assert( 0 );
}

static void data_receiver( void const* data, size_t size, void* user_data )
{
    (void)data;
    (void)size;
    (void)user_data;
    assert( 0 );
}

static void point_receiver( tobii_calibration_point_data_t const* point_data, void* user_data )
{
    (void)point_data;
    ++*(int*)user_data;
}

/* What the logger was called with: how often, and the last call's arguments. */
static int log_calls = 0;
static void* last_log_context = NULL;
static tobii_log_level_t last_log_level = TOBII_LOG_LEVEL_TRACE;
static char last_log_text[ 256 ] = { 0 };

static void log_func( void* log_context, tobii_log_level_t level, char const* text )
{
    ++log_calls;
    last_log_context = log_context;
    last_log_level = level;
    snprintf( last_log_text, sizeof( last_log_text ), "%s", text );
}

/* An allocator that counts its calls, in the int its mem_context points to,
 * and allocates nothing. */
static void* counting_malloc( void* mem_context, size_t size )
{
    (void)size;
    ++*(int*)mem_context;
    return NULL;
}

static void counting_free( void* mem_context, void* ptr )
{
    (void)ptr;
    ++*(int*)mem_context;
}

/* How far the system clock reads behind the wall clock at least: 1e15 us,
 * some 31.7 years, where the wall clock is some 1.8e15 us past its epoch and
 * CLOCK_MONOTONIC counts from boot. */
static int64_t const far_from_the_wall_clock_us = 1000000000000000LL;

static int64_t clock_us( clockid_t clock )
{
    struct timespec ts = { 0 };
    int const rc = clock_gettime( clock, &ts );
    assert( rc == 0 );
    (void)rc;
    return (int64_t)ts.tv_sec * 1000000 + ts.tv_nsec / 1000;
}

int main( void )
{
    size_t const exported = sizeof( abi_symbols ) / sizeof( abi_symbols[ 0 ] );
    assert( exported == 153 );
    for( size_t i = 0; i < exported; ++i )
        assert( abi_symbols[ i ] != NULL );

    /* The callback types must accept the plain functions clients write. */
    tobii_head_pose_callback_t const head_pose_cb = head_pose_callback;
    assert( head_pose_cb != NULL );

    tobii_version_t version;
    assert( tobii_get_api_version( &version ) == TOBII_ERROR_NO_ERROR );
    assert( version.major == 4 && version.minor == 1 && version.revision == 0 && version.build == 3 );

    tobii_api_t* api = NULL;
    assert( tobii_api_create( &api, NULL, NULL ) == TOBII_ERROR_NO_ERROR );
    assert( api != NULL );

    assert( tobii_enumerate_local_device_urls( api, url_receiver, &url_count ) == TOBII_ERROR_NO_ERROR );
    assert( url_count == 1 && first_url[ 0 ] != '\0' );
    assert( tobii_enumerate_local_device_urls( api, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_enumerate_local_device_urls_ex( api, url_receiver, &url_count, 0 )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_enumerate_local_device_urls_ex( api, url_receiver, &url_count, TOBII_DEVICE_GENERATION_IS4 )
        == TOBII_ERROR_NO_ERROR );

    /* The system clock is CLOCK_MONOTONIC, as the daemon's host timestamps
     * are: it reads between two readings of that, and far behind the wall
     * clock. */
    int64_t const before = clock_us( CLOCK_MONOTONIC );
    int64_t t0 = 0, t1 = 0;
    assert( tobii_system_clock( api, &t0 ) == TOBII_ERROR_NO_ERROR );
    assert( tobii_system_clock( api, &t1 ) == TOBII_ERROR_NO_ERROR );
    int64_t const after = clock_us( CLOCK_MONOTONIC );
    assert( before <= t0 && t0 <= t1 && t1 <= after );
    assert( clock_us( CLOCK_REALTIME ) - t1 > far_from_the_wall_clock_us );

    /* An out-of-range field_of_use is refused without writing the handle —
     * this is what a caller built against the pre-4.0 header trips over. */
    tobii_device_t* device = NULL;
    assert( tobii_device_create( api, first_url, (tobii_field_of_use_t)0, &device )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( device == NULL );
    tobii_license_validation_result_t results[ 2 ] = { 99, 99 };
    assert( tobii_device_create_ex( api, first_url, (tobii_field_of_use_t)0, NULL, 2, results, &device )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( results[ 0 ] == 99 && device == NULL );

    /* tobii_api_create checks custom_alloc and custom_log as the DLL does,
     * leaving the handle alone when it refuses them. The allocator is never
     * called; the logger hears libtobii's own diagnostics, here a refused
     * field_of_use, with its context. */
    int alloc_calls = 0;
    int log_context = 0;
    tobii_custom_alloc_t const custom_alloc = { &alloc_calls, counting_malloc, counting_free };
    tobii_custom_alloc_t const no_free = { &alloc_calls, counting_malloc, NULL };
    tobii_custom_log_t const custom_log = { &log_context, log_func };
    tobii_custom_log_t const no_log_func = { &log_context, NULL };
    static char untouched;
    tobii_api_t* logging = (tobii_api_t*)&untouched;
    assert( tobii_api_create( &logging, &no_free, &custom_log ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_api_create( &logging, &custom_alloc, &no_log_func ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( logging == (tobii_api_t*)&untouched );
    assert( tobii_api_create( &logging, &custom_alloc, &custom_log ) == TOBII_ERROR_NO_ERROR );
    assert( logging != NULL && logging != (tobii_api_t*)&untouched );
    assert( tobii_device_create( logging, first_url, (tobii_field_of_use_t)0, &device )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( log_calls == 1 && last_log_context == &log_context );
    assert( last_log_level == TOBII_LOG_LEVEL_ERROR && last_log_text[ 0 ] != '\0' );
    assert( tobii_api_destroy( logging ) == TOBII_ERROR_NO_ERROR );
    assert( alloc_calls == 0 && log_calls == 1 && device == NULL );

    /* Answered locally: the display area of a 597x336 mm monitor on the ET5
     * mounting is the one the Windows engine wrote (change-display.pcapng). */
    tobii_geometry_mounting_t const mounting = { 2, 184.0f, 20.0f, { 0.0f, -0.16f, 13.85f }, { 0.0f, 5.38f, 9.86f } };
    tobii_display_area_t area;
    assert( tobii_calculate_display_area_basic( api, 597.0f, 336.0f, 1.0022583f, &mounting, &area )
        == TOBII_ERROR_NO_ERROR );
    assert( fabsf( area.top_left_mm_xyz[ 0 ] + 297.49773f ) < 1e-3f );
    assert( fabsf( area.top_left_mm_xyz[ 1 ] - 326.00406f ) < 1e-3f );
    assert( fabsf( area.bottom_left_mm_xyz[ 2 ] + 3.10002f ) < 1e-3f );

    /* The DLL's own invalid calibration, a negative point count, in the
     * fewest bytes the argument checks pass: refused before any point. */
    unsigned char const negative_count[ 8 ] = { 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff };
    int points = 0;
    assert( tobii_calibration_parse( api, negative_count, sizeof( negative_count ), point_receiver, &points )
        == TOBII_ERROR_OPERATION_FAILED );
    assert( tobii_calibration_parse( api, negative_count, 7, point_receiver, &points )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( points == 0 );

    /* Unimplemented entry points answer, whatever they are given. */
    assert( tobii_open_realm( NULL, 0, NULL, 0 ) == TOBII_ERROR_NOT_SUPPORTED );

    /* Device-less calls are rejected before any device work. */
    assert( tobii_get_output_frequency( NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_calibration_start( NULL, TOBII_ENABLED_EYE_BOTH ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_calibration_discard_data_2d( NULL, 0.5f, 0.5f ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_calibration_collect_data_3d( NULL, 0.5f, 0.5f, 0.5f ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_calibration_discard_data_3d( NULL, 0.5f, 0.5f, 0.5f ) == TOBII_ERROR_INVALID_PARAMETER );
    tobii_enabled_eye_t eyes = (tobii_enabled_eye_t)7;
    assert( tobii_calibration_collect_data_per_eye_2d( NULL, 0.5f, 0.5f, TOBII_ENABLED_EYE_LEFT, &eyes )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_calibration_discard_data_per_eye_2d( NULL, 0.5f, 0.5f, TOBII_ENABLED_EYE_LEFT )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_calibration_compute_and_apply_per_eye( NULL, &eyes ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( eyes == (tobii_enabled_eye_t)7 );
    tobii_supported_t supported = TOBII_SUPPORTED;
    assert( tobii_internal_stream_supported( NULL, 0, &supported ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_internal_capability_supported( NULL, 0, &supported ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_gaze_raw_subscribe( NULL, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_gaze_raw_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_foveated_rendering_gaze_point_subscribe( NULL, foveated_callback, NULL )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_foveated_rendering_gaze_point_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_custom_stream_subscribe( NULL, custom_stream_callback, 0, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_custom_stream_subscribe( NULL, NULL, 0, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_custom_stream_unsubscribe( NULL, 0 ) == TOBII_ERROR_INVALID_PARAMETER );
    /* Refused internal streams: any non-null callback, as it is never called. */
    assert( tobii_low_frequency_head_rotation_subscribe( NULL, &supported, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_low_frequency_head_rotation_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_low_frequency_head_position_subscribe( NULL, &supported, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_low_frequency_head_position_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_multiple_faces_position_subscribe( NULL, &supported, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_multiple_faces_position_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_wearable_limited_image_subscribe( NULL, &supported, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_wearable_limited_image_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_secondary_camera_image_subscribe( NULL, &supported, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_secondary_camera_image_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_low_frequency_head_position_subscribe( NULL, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    /* Wearable entry points: TOBII_ERROR_NOT_SUPPORTED once their arguments
     * check out, as the DLL answers for an ET5, so a NULL device or argument
     * is refused, and nothing is written. */
    assert( tobii_wearable_consumer_data_subscribe( NULL, wearable_consumer_callback, NULL )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_wearable_consumer_data_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_wearable_advanced_data_subscribe( NULL, wearable_advanced_callback, NULL )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_wearable_advanced_data_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_wearable_foveated_gaze_subscribe( NULL, wearable_foveated_callback, NULL )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_wearable_foveated_gaze_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_wearable_consumer_data_subscribe( NULL, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    tobii_lens_configuration_t lens = { { 7.0f, 7.0f, 7.0f }, { 7.0f, 7.0f, 7.0f } };
    assert( tobii_get_lens_configuration( NULL, &lens ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_get_lens_configuration( NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_set_lens_configuration( NULL, &lens ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_set_lens_configuration( NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( lens.left_xyz[ 0 ] == 7.0f && lens.right_xyz[ 2 ] == 7.0f );
    /* The sync port and face types alike; the DLL never checks the sync
     * port's callback. */
    assert( tobii_digital_syncport_subscribe( NULL, syncport_callback, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_digital_syncport_subscribe( NULL, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_digital_syncport_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_enumerate_face_types( NULL, face_type_receiver, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    tobii_face_type_t face_type = "unchanged";
    assert( tobii_set_face_type( NULL, face_type ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_get_face_type( NULL, &face_type ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_get_face_type( NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( strcmp( face_type, "unchanged" ) == 0 );
    /* Licence storage alike; the store never reads its data or size. */
    assert( tobii_license_key_store( NULL, NULL, 0 ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_license_key_retrieve( NULL, data_receiver, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_license_key_retrieve( NULL, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    /* Face id alike, through its typed prototypes. */
    uint32_t const face_id = 7;
    uint32_t enrolled = 7;
    tobii_face_id_state_t face_id_state = { 7, 7 };
    tobii_face_id_parameters_t face_id_parameters = { 7 };
    assert( tobii_face_id_enroll( NULL, &face_id, &enrolled ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_face_id_enroll_clear( NULL, &face_id ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_face_id_state_subscribe( NULL, face_id_state_callback, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_face_id_state_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_face_id_parameters_subscribe( NULL, face_id_parameters_callback, NULL )
        == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_face_id_parameters_unsubscribe( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_get_face_id_state( NULL, &face_id_state ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_get_face_id_parameters( NULL, &face_id_parameters ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_set_face_id_parameters( NULL, &face_id_parameters ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_face_id_enroll( NULL, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( enrolled == 7 && face_id_state.state == 7 && face_id_state.value == 7 && face_id_parameters.value == 7 );
    /* Statistics take two strings; the display information and the firmware
     * upgrade switch need no device to answer as the DLL does. */
    assert( tobii_send_statistics( NULL, "a", "b" ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_send_statistics( NULL, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    char const display_info[ 72 ] = { 0 };
    assert( tobii_set_display_info( NULL, display_info ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_set_fw_upgrade_allowed( NULL, 1 ) == TOBII_ERROR_NOT_SUPPORTED );
    tobii_lens_configuration_writable_t writable = TOBII_LENS_CONFIGURATION_WRITABLE;
    assert( tobii_lens_configuration_writable( NULL, &writable ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( supported == TOBII_SUPPORTED && writable == TOBII_LENS_CONFIGURATION_WRITABLE );
    tobii_timesync_data_t timesync = { 1, 2, 3 };
    assert( tobii_timesync( NULL, &timesync ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( timesync.system_start_us == 1 && timesync.tracker_us == 3 );
    assert( tobii_enumerate_stream_types( NULL, NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_enumerate_stream_type_columns( NULL, 1, column_receiver, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_pause_device( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    tobii_hardware_configuration_t hardware = { .entry_count = 7, .mode = 7 };
    assert( tobii_hardware_configuration_get( NULL, &hardware ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( hardware.entry_count == 7 && hardware.mode == 7 );
    tobii_device_info_internal_t info_internal = { "unchanged", "unchanged" };
    assert( tobii_get_device_info_internal( NULL, &info_internal ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( strcmp( info_internal.platform_type, "unchanged" ) == 0 );
    tobii_calibration_stimulus_points_t stimulus = { .point_count = 7, .points = { { .words = { 7 } } } };
    assert( tobii_calibration_stimulus_points_get( NULL, &stimulus ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_calibration_stimulus_points_get( NULL, NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( stimulus.point_count == 7 && stimulus.points[ 0 ].words[ 0 ] == 7 );
    assert( tobii_resume_device( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    tobii_device_name_t name = "unchanged";
    assert( tobii_set_device_name( NULL, name ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_get_device_name( NULL, &name ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( strcmp( name, "unchanged" ) == 0 );
    assert( tobii_device_destroy( NULL ) == TOBII_ERROR_INVALID_PARAMETER );
    assert( tobii_api_destroy( NULL ) == TOBII_ERROR_INVALID_PARAMETER );

    /* The DLL's texts; 2 and 19 share one. Out of range, a negative code
     * included, is the DLL's generic text. */
    assert( strcmp( tobii_error_message( TOBII_ERROR_NO_ERROR ), "No error." ) == 0 );
    assert( strcmp( tobii_error_message( TOBII_ERROR_INSUFFICIENT_LICENSE ),
        tobii_error_message( TOBII_ERROR_UNAUTHORIZED ) ) == 0 );
    assert( strcmp( tobii_error_message( (tobii_error_t)9999 ),
        "Undefined error (0x270f). Please contact support." ) == 0 );
    assert( strcmp( tobii_error_message( (tobii_error_t)-1 ),
        "Undefined error (0xffffffff). Please contact support." ) == 0 );

    /* Enum numbering is the Stream Engine's. */
    assert( TOBII_ERROR_CONNECTION_FAILED == 5 && TOBII_ERROR_INVALID_PARAMETER == 8 );
    assert( TOBII_ERROR_CONFLICTING_API_INSTANCES == 14 && TOBII_ERROR_FIRMWARE_UPGRADE_IN_PROGRESS == 20 );
    assert( TOBII_CAPABILITY_COMPOUND_STREAM_WEARABLE_INCREASE_EYE_RELIEF == 18 );
    _Static_assert( TOBII_STREAM_GAZE_POINT == 0 && TOBII_STREAM_GAZE_ORIGIN == 1
            && TOBII_STREAM_EYE_POSITION_NORMALIZED == 2 && TOBII_STREAM_USER_PRESENCE == 3,
        "tobii_stream_t is numbered as the 4.1 DLL numbers it" );
    _Static_assert( TOBII_STREAM_HEAD_POSE == 4 && TOBII_STREAM_GAZE_DATA == 5
            && TOBII_STREAM_DIGITAL_SYNCPORT == 6 && TOBII_STREAM_DIAGNOSTICS_IMAGE == 7,
        "tobii_stream_t is numbered as the 4.1 DLL numbers it" );
    _Static_assert( TOBII_STREAM_USER_POSITION_GUIDE == 8 && TOBII_STREAM_WEARABLE_CONSUMER == 9
            && TOBII_STREAM_WEARABLE_ADVANCED == 10 && TOBII_STREAM_WEARABLE_FOVEATED_GAZE == 11,
        "tobii_stream_t is numbered as the 4.1 DLL numbers it" );
    assert( TOBII_STATE_CALIBRATION_ACTIVE == 7 );
    assert( TOBII_LOG_LEVEL_ERROR == 0 && TOBII_LOG_LEVEL_INFO == 2 && TOBII_LOG_LEVEL_TRACE == 4 );
    assert( TOBII_NOTIFICATION_TYPE_FACE_TYPE_CHANGED == 12 && TOBII_NOTIFICATION_VALUE_TYPE_STRING == 6 );
    assert( TOBII_LENS_CONFIGURATION_NOT_WRITABLE == 0 && TOBII_LENS_CONFIGURATION_WRITABLE == 1 );
    _Static_assert( TOBII_CALIBRATION_POINT_STATUS_FAILED_OR_INVALID == 0
            && TOBII_CALIBRATION_POINT_STATUS_VALID_BUT_NOT_USED_IN_CALIBRATION == 1
            && TOBII_CALIBRATION_POINT_STATUS_VALID_AND_USED_IN_CALIBRATION == 2,
        "numbered as tobii_calibration_parse writes them" );

    /* Struct layouts are shared memory between this program and the library;
     * the same numbers are pinned in the Rust tests. */
    assert( sizeof( tobii_device_info_t ) == 2048 );
    assert( sizeof( tobii_device_info_internal_t ) == 256 );
    assert( offsetof( tobii_device_info_internal_t, property_4 ) == 0x80 );
    assert( offsetof( tobii_device_info_t, model ) == 256 );
    assert( offsetof( tobii_device_info_t, runtime_build_version ) == 0x700 );
    assert( sizeof( tobii_track_box_t ) == 96 );
    assert( sizeof( tobii_display_area_t ) == 36 );
    assert( sizeof( tobii_geometry_mounting_t ) == 36 );
    assert( sizeof( tobii_head_pose_t ) == 48 );
    assert( offsetof( tobii_head_pose_t, rotation_xyz ) == 36 );
    assert( sizeof( tobii_gaze_point_t ) == 24 );
    assert( sizeof( tobii_gaze_origin_t ) == 40 );
    assert( sizeof( tobii_eye_position_normalized_t ) == 40 );
    assert( sizeof( tobii_user_position_guide_t ) == 40 );
    assert( sizeof( tobii_gaze_data_eye_t ) == 76 );
    assert( sizeof( tobii_gaze_data_t ) == 168 );
    assert( offsetof( tobii_gaze_data_t, right ) == 92 );
    assert( sizeof( tobii_notification_t ) == 520 );
    assert( offsetof( tobii_notification_t, value ) == 8 );
    assert( sizeof( tobii_image_t ) == 32 );
    assert( offsetof( tobii_image_t, data ) == 24 );
    assert( sizeof( tobii_gaze_raw_eye_t ) == 52 );
    assert( offsetof( tobii_gaze_raw_eye_t, status ) == 48 );
    assert( sizeof( tobii_gaze_raw_t ) == 232 );
    assert( _Alignof( tobii_gaze_raw_t ) == 8 );
    assert( offsetof( tobii_gaze_raw_t, right ) == 0x3c );
    assert( offsetof( tobii_gaze_raw_t, combined_gaze_validity ) == 0x78 );
    assert( offsetof( tobii_gaze_raw_t, key_0e_validity ) == 0x7c );
    assert( offsetof( tobii_gaze_raw_t, key_11 ) == 0x98 );
    assert( offsetof( tobii_gaze_raw_t, frame_counter ) == 0xb0 );
    assert( offsetof( tobii_gaze_raw_t, right_origin_flag ) == 0xc0 );
    assert( offsetof( tobii_gaze_raw_t, left_eyeball_center_validity ) == 0xc4 );
    assert( offsetof( tobii_gaze_raw_t, right_eyeball_center_from_eye_tracker_mm_xyz ) == 0xd8 );
    assert( offsetof( tobii_gaze_raw_t, reserved_e4 ) == 0xe4 );
    assert( sizeof( tobii_wearable_eye_t ) == 32 );
    assert( sizeof( tobii_wearable_consumer_data_t ) == 120 );
    assert( offsetof( tobii_wearable_consumer_data_t, right ) == 0x28 );
    assert( sizeof( tobii_wearable_advanced_eye_t ) == 72 );
    assert( offsetof( tobii_wearable_advanced_eye_t, pupil_diameter_mm ) == 0x24 );
    assert( offsetof( tobii_wearable_advanced_eye_t, blink ) == 0x44 );
    assert( sizeof( tobii_wearable_advanced_data_t ) == 208 );
    assert( offsetof( tobii_wearable_advanced_data_t, right ) == 0x58 );
    assert( offsetof( tobii_wearable_advanced_data_t, gaze_origin_combined_validity ) == 0xa0 );
    assert( offsetof( tobii_wearable_advanced_data_t, increase_eye_relief ) == 0xcc );
    assert( sizeof( tobii_lens_configuration_t ) == 24 );
    assert( sizeof( tobii_wearable_foveated_gaze_t ) == 24 );
    assert( sizeof( tobii_face_id_state_t ) == 8 );
    assert( sizeof( tobii_face_id_parameters_t ) == 4 );
    assert( sizeof( tobii_timesync_data_t ) == 24 );
    assert( offsetof( tobii_timesync_data_t, system_end_us ) == 8 );
    assert( offsetof( tobii_timesync_data_t, tracker_us ) == 16 );
    assert( sizeof( tobii_stream_type_t ) == 136 );
    assert( offsetof( tobii_stream_type_t, value ) == 4 );
    assert( offsetof( tobii_stream_type_t, name ) == 8 );
    assert( offsetof( tobii_stream_type_t, text ) == 72 );
    assert( sizeof( tobii_stream_type_column_t ) == 140 );
    assert( _Alignof( tobii_stream_type_column_t ) == 4 );
    assert( offsetof( tobii_stream_type_column_t, param_a ) == 4 );
    assert( offsetof( tobii_stream_type_column_t, param_b ) == 8 );
    assert( offsetof( tobii_stream_type_column_t, name ) == 12 );
    assert( offsetof( tobii_stream_type_column_t, text ) == 76 );
    assert( sizeof( tobii_custom_stream_data_t ) == 24 );
    assert( _Alignof( tobii_custom_stream_data_t ) == 8 );
    assert( offsetof( tobii_custom_stream_data_t, reserved_04 ) == 4 );
    assert( offsetof( tobii_custom_stream_data_t, size ) == 8 );
    assert( offsetof( tobii_custom_stream_data_t, data ) == 16 );
    assert( sizeof( tobii_hardware_configuration_entry_t ) == 0x2e8 );
    assert( offsetof( tobii_hardware_configuration_entry_t, position_xyz ) == 0x10 );
    assert( offsetof( tobii_hardware_configuration_entry_t, width ) == 0xa0 );
    assert( offsetof( tobii_hardware_configuration_entry_t, coefficients ) == 0xb0 );
    assert( offsetof( tobii_hardware_configuration_entry_t, param_d ) == 0x2e0 );
    assert( sizeof( tobii_hardware_configuration_t ) == 2472 );
    assert( offsetof( tobii_hardware_configuration_t, entries ) == 8 );
    assert( offsetof( tobii_hardware_configuration_t, point_count ) == 0x5d8 );
    assert( offsetof( tobii_hardware_configuration_t, points_xyz ) == 0x5e0 );
    assert( offsetof( tobii_hardware_configuration_t, mode ) == 0x9a0 );
    assert( sizeof( tobii_calibration_point_data_t ) == 32 );
    assert( sizeof( tobii_calibration_stimulus_point_t ) == 36 );
    assert( sizeof( tobii_calibration_stimulus_points_t ) == 1156 );
    assert( _Alignof( tobii_calibration_stimulus_points_t ) == 4 );
    assert( offsetof( tobii_calibration_stimulus_points_t, points ) == 4 );
    assert( sizeof( tobii_license_key_t ) == 16 );
    assert( sizeof( tobii_custom_alloc_t ) == 24 );
    assert( offsetof( tobii_custom_alloc_t, malloc_func ) == 8 );
    assert( offsetof( tobii_custom_alloc_t, free_func ) == 16 );
    assert( sizeof( tobii_custom_log_t ) == 16 );
    assert( offsetof( tobii_custom_log_t, log_func ) == 8 );
    assert( sizeof( tobii_device_name_t ) == 64 );
    assert( sizeof( tobii_state_string_t ) == 512 );

    assert( tobii_api_destroy( api ) == TOBII_ERROR_NO_ERROR );

    printf( "abi-smoke: ok (%zu symbols, device url \"%s\")\n", exported, first_url );
    return 0;
}
