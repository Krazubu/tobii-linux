/* tobii/tobii_signing.h — SHA-256 with RSA PKCS#1 v1.5 over a PEM key, as
 * provided by libtobii.so. The four entry points Stream Engine 4.25 adds.
 * Companion to tobii/tobii.h.
 *
 * Nothing here touches a device: no tobii_device_t, no tobii_api_t, no
 * licence. The DLL does this work in statically linked OpenSSL; libtobii.so
 * does it in Rust, with the same scheme and the same base64 wrapping.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef tobii_signing_h_included
#define tobii_signing_h_included

#include "tobii.h"

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* These four do NOT return tobii_error_t. They return this enumeration,
 * whose values run 0..16 — and every one of them is also a valid
 * tobii_error_t value (2 is TOBII_ERROR_INSUFFICIENT_LICENSE, 8 is
 * TOBII_ERROR_INVALID_PARAMETER, 16 is TOBII_ERROR_CALLBACK_IN_PROGRESS), so
 * a caller that maps a result through the error table gets wrong semantics
 * with no out-of-range value to catch it. Check against these names.
 *
 * The values 7..12 and 14..15 name failures inside the DLL's OpenSSL calls
 * that libtobii.so cannot produce; they are declared so the numbering is
 * whole. The names are ours: the DLL names none of them. */
typedef enum tobii_signing_result_t
{
    TOBII_SIGNING_OK,                       /* 0 */
    TOBII_SIGNING_KEY_BUFFER_FAILED,        /* 1 */
    TOBII_SIGNING_NULL_ARGUMENT,            /* 2 */
    TOBII_SIGNING_SIGN_OPEN_FAILED,         /* 3 */
    TOBII_SIGNING_SIGN_PEM_FAILED,          /* 4 */
    TOBII_SIGNING_VERIFY_OPEN_FAILED,       /* 5 */
    TOBII_SIGNING_VERIFY_PEM_FAILED,        /* 6 */
    TOBII_SIGNING_DIGEST_CONTEXT_FAILED,    /* 7 */
    TOBII_SIGNING_DIGEST_FAILED,            /* 8 */
    TOBII_SIGNING_SIGN_INIT_FAILED,         /* 9 */
    TOBII_SIGNING_SIGN_UPDATE_FAILED,       /* 10 */
    TOBII_SIGNING_SIGN_FINAL_FAILED,        /* 11 */
    TOBII_SIGNING_ALLOCATION_FAILED,        /* 12 */
    TOBII_SIGNING_BASE64_FAILED,            /* 13 */
    TOBII_SIGNING_VERIFY_INIT_FAILED,       /* 14 */
    TOBII_SIGNING_VERIFY_UPDATE_FAILED,     /* 15 */
    TOBII_SIGNING_NOT_VERIFIED,             /* 16 */
} tobii_signing_result_t;

/* Sign data_size bytes of data with the PEM private key (PKCS#8 or PKCS#1)
 * in the file key_path, or held in key_data. The base64 signature goes to
 * signature, which must point to at least 640 bytes: the call takes no
 * capacity, and 640 is what the DLL writes. Its length, without the
 * terminator, goes to signature_size.
 *
 * libtobii.so NUL-terminates the signature whenever it writes one and writes
 * nothing at all on failure. The DLL instead memsets 640 bytes before it has
 * read anything, terminates only while the encoded length is 477 or less, and
 * never terminates in the _with_private_key form, so a caller written against
 * the DLL must use signature_size. A null argument here is
 * TOBII_SIGNING_NULL_ARGUMENT, where the DLL dereferences it. */
TOBII_API tobii_signing_result_t TOBII_CALL tobii_signing_sign( void const* data,
    size_t data_size, char* signature, size_t* signature_size, char const* key_path );
TOBII_API tobii_signing_result_t TOBII_CALL tobii_signing_sign_with_private_key(
    void const* data, size_t data_size, char* signature, size_t* signature_size,
    void const* key_data, size_t key_data_size );

/* Verify signature_size bytes of the base64 signature over data_size bytes of
 * data, under the PEM public key (SPKI) in the file key_path or held in
 * key_data: TOBII_SIGNING_OK, or TOBII_SIGNING_NOT_VERIFIED when the
 * signature does not stand. */
TOBII_API tobii_signing_result_t TOBII_CALL tobii_signing_verify( void const* data,
    size_t data_size, char const* signature, size_t signature_size, char const* key_path );
TOBII_API tobii_signing_result_t TOBII_CALL tobii_signing_verify_with_public_key(
    void const* data, size_t data_size, char const* signature, size_t signature_size,
    void const* key_data, size_t key_data_size );

#ifdef __cplusplus
}
#endif

#endif /* tobii_signing_h_included */
