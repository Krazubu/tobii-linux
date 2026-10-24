//! The four signing entry points Stream Engine 4.25 adds: SHA-256 with RSA
//! PKCS#1 v1.5 over a PEM key, the signature in standard base64.
//!
//! No device, no api handle, no protocol id and no licence gate: the DLL does
//! this work itself, in statically linked OpenSSL 1.1.1o, from
//! `foundation_libs/tobii_signing/src/tobii_signing.cpp`. Here it is the
//! `rsa`, `sha2` and `base64` crates, so `libtobii.so` keeps its system
//! dependencies. `pctx` is NULL in both of the DLL's init calls and nothing
//! in its subsystem calls `EVP_PKEY_CTX_ctrl`, so the padding is OpenSSL's
//! default `RSA_PKCS1_PADDING`, which is what PKCS#1 v1.5 here means.
//!
//! These four do **not** return `tobii_error_t`. They return a dedicated
//! 32-bit result whose values run 0..=16, and every one of them is also a
//! valid `tobii_error_t` — 2 reads as `TOBII_ERROR_INSUFFICIENT_LICENSE`, 8
//! as `TOBII_ERROR_INVALID_PARAMETER`, 16 as
//! `TOBII_ERROR_CALLBACK_IN_PROGRESS` — so a caller that maps them through
//! the error table gets silently wrong semantics with no out-of-range value
//! to catch it. The DLL's own caller collapses the result to a boolean.
//!
//! Three of the DLL's behaviours are deliberately **not** reproduced, because
//! each is a memory error rather than a contract: `tobii_signing_sign`
//! memsets 640 bytes of the caller's buffer before it has read anything and
//! stores through `signature_size` without checking it; `tobii_signing_verify`
//! decodes into a 640-byte frame under a bound the caller supplies, so a
//! signature of 856 bytes or more runs off it; and `tobii_signing_verify`
//! calls `fopen(NULL, "r")` for a null key path. Here every pointer is
//! checked first (`TOBII_SIGNING_NULL_ARGUMENT`), nothing is written before
//! the work succeeds, and an encoded signature that does not fit the
//! documented 640-byte buffer is `TOBII_SIGNING_BASE64_FAILED` instead of an
//! overrun. The DLL also leaks the `EVP_PKEY` of both file-path variants;
//! nothing here can leak.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::{fs, slice};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::{Signature, SigningKey, VerifyingKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::signature::{SignatureEncoding, Signer, Verifier};
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::Sha256;

/// `tobii_signing_result_t`: the result of the four signing entry points,
/// which is **not** a `tobii_error_t` (see the module documentation).
pub type SigningResult = c_int;

/// Signed or verified.
pub const TOBII_SIGNING_OK: SigningResult = 0;
/// The key bytes could not be taken as a key at all (the DLL's
/// `BIO_new_mem_buf` failure).
pub const TOBII_SIGNING_KEY_BUFFER_FAILED: SigningResult = 1;
/// An argument the call requires was null.
pub const TOBII_SIGNING_NULL_ARGUMENT: SigningResult = 2;
/// Signing: the key file could not be opened.
pub const TOBII_SIGNING_SIGN_OPEN_FAILED: SigningResult = 3;
/// Signing: the key file held no PEM private key.
pub const TOBII_SIGNING_SIGN_PEM_FAILED: SigningResult = 4;
/// Verifying: the key file could not be opened.
pub const TOBII_SIGNING_VERIFY_OPEN_FAILED: SigningResult = 5;
/// Verifying: the key file held no PEM public key.
pub const TOBII_SIGNING_VERIFY_PEM_FAILED: SigningResult = 6;
/// The digest context could not be created.
#[allow(
    dead_code,
    reason = "used only by the C header and the tests; a value of the DLL's enum that its OpenSSL path can return and this implementation cannot; declared so the C header carries the whole numbering"
)]
pub const TOBII_SIGNING_DIGEST_CONTEXT_FAILED: SigningResult = 7;
/// The digest could not be looked up by name.
#[allow(
    dead_code,
    reason = "used only by the C header and the tests; a value of the DLL's enum that its OpenSSL path can return and this implementation cannot; declared so the C header carries the whole numbering"
)]
pub const TOBII_SIGNING_DIGEST_FAILED: SigningResult = 8;
/// The signing operation could not be initialised.
#[allow(
    dead_code,
    reason = "used only by the C header and the tests; a value of the DLL's enum that its OpenSSL path can return and this implementation cannot; declared so the C header carries the whole numbering"
)]
pub const TOBII_SIGNING_SIGN_INIT_FAILED: SigningResult = 9;
/// The data could not be fed to the signing operation.
#[allow(
    dead_code,
    reason = "used only by the C header and the tests; a value of the DLL's enum that its OpenSSL path can return and this implementation cannot; declared so the C header carries the whole numbering"
)]
pub const TOBII_SIGNING_SIGN_UPDATE_FAILED: SigningResult = 0xa;
/// The signature could not be produced.
#[allow(
    dead_code,
    reason = "used only by the C header and the tests; a value of the DLL's enum that its OpenSSL path can return and this implementation cannot; declared so the C header carries the whole numbering"
)]
pub const TOBII_SIGNING_SIGN_FINAL_FAILED: SigningResult = 0xb;
/// The signature buffer could not be allocated.
#[allow(
    dead_code,
    reason = "used only by the C header and the tests; a value of the DLL's enum that its OpenSSL path can return and this implementation cannot; declared so the C header carries the whole numbering"
)]
pub const TOBII_SIGNING_ALLOCATION_FAILED: SigningResult = 0xc;
/// The signature could not be encoded, or does not fit the caller's buffer.
pub const TOBII_SIGNING_BASE64_FAILED: SigningResult = 0xd;
/// The verifying operation could not be initialised.
#[allow(
    dead_code,
    reason = "used only by the C header and the tests; a value of the DLL's enum that its OpenSSL path can return and this implementation cannot; declared so the C header carries the whole numbering"
)]
pub const TOBII_SIGNING_VERIFY_INIT_FAILED: SigningResult = 0xe;
/// The data could not be fed to the verifying operation.
#[allow(
    dead_code,
    reason = "used only by the C header and the tests; a value of the DLL's enum that its OpenSSL path can return and this implementation cannot; declared so the C header carries the whole numbering"
)]
pub const TOBII_SIGNING_VERIFY_UPDATE_FAILED: SigningResult = 0xf;
/// The signature does not verify under the key.
pub const TOBII_SIGNING_NOT_VERIFIED: SigningResult = 0x10;

/// The size of the signature buffer the DLL writes into, and therefore the
/// size a caller of [`tobii_signing_sign`] has to provide: it memsets this
/// many bytes before signing and never asks the caller how many there are.
/// It is also the size of the frame its verify decodes into.
const SIGNATURE_BUFFER: usize = 0x280;

/// What a key source yielded: the PEM text, or the result to answer with.
enum Pem {
    Text(String),
    Failed(SigningResult),
}

/// The PEM text of the key file at `path`, as the DLL's `fopen`/`PEM_read`
/// pair reads it: `open` for a path that cannot be opened or is not UTF-8
/// (PEM is ASCII, so a file that is not UTF-8 holds no PEM), `pem` for one
/// whose contents no key could be read from.
fn pem_from_path(path: &CStr, open: SigningResult, pem: SigningResult) -> Pem {
    let Ok(path) = path.to_str() else {
        return Pem::Failed(open);
    };
    match fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Pem::Text(text),
            Err(_) => Pem::Failed(pem),
        },
        Err(error) => {
            tracing::trace!(%error, "signing: the key file could not be read");
            Pem::Failed(open)
        }
    }
}

/// The PEM text of a key held in memory, as the DLL's `BIO_new_mem_buf` pair
/// reads it: bytes that are not UTF-8 hold no PEM, which is the buffer
/// failure the DLL answers before it reaches `PEM_read_bio`.
fn pem_from_memory(bytes: &[u8]) -> Pem {
    match std::str::from_utf8(bytes) {
        Ok(text) => Pem::Text(text.to_owned()),
        Err(_) => Pem::Failed(TOBII_SIGNING_KEY_BUFFER_FAILED),
    }
}

/// A private key from PEM, PKCS#8 first and then PKCS#1, as OpenSSL's
/// `PEM_read_bio_PrivateKey` accepts either.
fn private_key(pem: &str) -> Option<RsaPrivateKey> {
    RsaPrivateKey::from_pkcs8_pem(pem)
        .ok()
        .or_else(|| RsaPrivateKey::from_pkcs1_pem(pem).ok())
}

/// Sign `data` with the private key in `pem` and write the base64 signature
/// to `signature`, with its length (the encoded length, without the
/// terminator) to `signature_size`.
///
/// # Safety
/// `signature` must be valid for writing [`SIGNATURE_BUFFER`] bytes and
/// `signature_size` for one `usize`; both are checked non-null by the caller.
unsafe fn sign_with(
    pem: &str,
    data: &[u8],
    signature: *mut c_char,
    signature_size: *mut usize,
    pem_failed: SigningResult,
) -> SigningResult {
    let Some(key) = private_key(pem) else {
        return pem_failed;
    };
    let encoded = STANDARD.encode(SigningKey::<Sha256>::new(key).sign(data).to_vec());
    // The DLL would run off the caller's buffer here; this does not.
    if encoded.len() >= SIGNATURE_BUFFER {
        tracing::trace!(
            len = encoded.len(),
            "signing: the encoded signature does not fit the caller's buffer"
        );
        return TOBII_SIGNING_BASE64_FAILED;
    }
    // SAFETY: the caller guarantees `signature` is valid for
    // `SIGNATURE_BUFFER` bytes, and `encoded` is shorter than that, so the
    // bytes and the terminator fit.
    unsafe {
        let out = slice::from_raw_parts_mut(signature.cast::<u8>(), encoded.len() + 1);
        out[..encoded.len()].copy_from_slice(encoded.as_bytes());
        out[encoded.len()] = 0;
        signature_size.write(encoded.len());
    }
    TOBII_SIGNING_OK
}

/// Verify `signature` over `data` under the public key in `pem`.
fn verify_with(
    pem: &str,
    data: &[u8],
    signature: &[u8],
    pem_failed: SigningResult,
) -> SigningResult {
    let Ok(key) = RsaPublicKey::from_public_key_pem(pem) else {
        return pem_failed;
    };
    let Ok(raw) = STANDARD.decode(signature) else {
        return TOBII_SIGNING_BASE64_FAILED;
    };
    // The DLL's frame is this big and its bound comes from the caller, so a
    // longer signature overruns it there. Here it is simply not a signature.
    if raw.len() > SIGNATURE_BUFFER {
        return TOBII_SIGNING_BASE64_FAILED;
    }
    let Ok(parsed) = Signature::try_from(&raw[..]) else {
        return TOBII_SIGNING_NOT_VERIFIED;
    };
    match VerifyingKey::<Sha256>::new(key).verify(data, &parsed) {
        Ok(()) => TOBII_SIGNING_OK,
        Err(_) => TOBII_SIGNING_NOT_VERIFIED,
    }
}

/// Sign `data_size` bytes of `data` with the PEM private key in the file
/// `key_path`, writing the base64 signature to `signature` and its length to
/// `signature_size`.
///
/// `signature` must point to [`SIGNATURE_BUFFER`] bytes: the call takes no
/// capacity and the DLL writes that many. The signature is NUL-terminated
/// here whenever it is written, which the DLL only manages for an encoded
/// length of 477 or less.
///
/// # Safety
/// `data` must be null or valid for reading `data_size` bytes; `signature`
/// must be null or valid for writing [`SIGNATURE_BUFFER`] bytes;
/// `signature_size` must be null or valid for writing one `usize`;
/// `key_path` must be null or a NUL-terminated path. A null argument is
/// `TOBII_SIGNING_NULL_ARGUMENT` and nothing is read or written.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_signing_sign(
    data: *const c_void,
    data_size: usize,
    signature: *mut c_char,
    signature_size: *mut usize,
    key_path: *const c_char,
) -> SigningResult {
    if data.is_null() || signature.is_null() || signature_size.is_null() || key_path.is_null() {
        return TOBII_SIGNING_NULL_ARGUMENT;
    }
    // SAFETY: non-null, and the caller guarantees the extents.
    let (bytes, path) = unsafe {
        (
            slice::from_raw_parts(data.cast::<u8>(), data_size),
            CStr::from_ptr(key_path),
        )
    };
    match pem_from_path(
        path,
        TOBII_SIGNING_SIGN_OPEN_FAILED,
        TOBII_SIGNING_SIGN_PEM_FAILED,
    ) {
        Pem::Failed(result) => result,
        // SAFETY: both pointers are non-null and valid as the caller
        // guarantees.
        Pem::Text(pem) => unsafe {
            sign_with(
                &pem,
                bytes,
                signature,
                signature_size,
                TOBII_SIGNING_SIGN_PEM_FAILED,
            )
        },
    }
}

/// As [`tobii_signing_sign`], with the PEM private key given as
/// `key_data_size` bytes of `key_data` instead of a file.
///
/// # Safety
/// As [`tobii_signing_sign`], and `key_data` must be null or valid for
/// reading `key_data_size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_signing_sign_with_private_key(
    data: *const c_void,
    data_size: usize,
    signature: *mut c_char,
    signature_size: *mut usize,
    key_data: *const c_void,
    key_data_size: usize,
) -> SigningResult {
    if data.is_null() || signature.is_null() || signature_size.is_null() || key_data.is_null() {
        return TOBII_SIGNING_NULL_ARGUMENT;
    }
    // SAFETY: non-null, and the caller guarantees the extents.
    let (bytes, key) = unsafe {
        (
            slice::from_raw_parts(data.cast::<u8>(), data_size),
            slice::from_raw_parts(key_data.cast::<u8>(), key_data_size),
        )
    };
    match pem_from_memory(key) {
        Pem::Failed(result) => result,
        // SAFETY: both pointers are non-null and valid as the caller
        // guarantees.
        Pem::Text(pem) => unsafe {
            sign_with(
                &pem,
                bytes,
                signature,
                signature_size,
                TOBII_SIGNING_SIGN_PEM_FAILED,
            )
        },
    }
}

/// Verify `signature_size` bytes of the base64 `signature` over `data_size`
/// bytes of `data`, under the PEM public key in the file `key_path`:
/// [`TOBII_SIGNING_OK`] or [`TOBII_SIGNING_NOT_VERIFIED`].
///
/// # Safety
/// `data` must be null or valid for reading `data_size` bytes; `signature`
/// must be null or valid for reading `signature_size` bytes; `key_path` must
/// be null or a NUL-terminated path. A null argument is
/// `TOBII_SIGNING_NULL_ARGUMENT`, where the DLL would `fopen(NULL, "r")`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_signing_verify(
    data: *const c_void,
    data_size: usize,
    signature: *const c_char,
    signature_size: usize,
    key_path: *const c_char,
) -> SigningResult {
    if data.is_null() || signature.is_null() || key_path.is_null() {
        return TOBII_SIGNING_NULL_ARGUMENT;
    }
    // SAFETY: non-null, and the caller guarantees the extents.
    let (bytes, sig, path) = unsafe {
        (
            slice::from_raw_parts(data.cast::<u8>(), data_size),
            slice::from_raw_parts(signature.cast::<u8>(), signature_size),
            CStr::from_ptr(key_path),
        )
    };
    match pem_from_path(
        path,
        TOBII_SIGNING_VERIFY_OPEN_FAILED,
        TOBII_SIGNING_VERIFY_PEM_FAILED,
    ) {
        Pem::Failed(result) => result,
        Pem::Text(pem) => verify_with(&pem, bytes, sig, TOBII_SIGNING_VERIFY_PEM_FAILED),
    }
}

/// As [`tobii_signing_verify`], with the PEM public key given as
/// `key_data_size` bytes of `key_data` instead of a file.
///
/// # Safety
/// As [`tobii_signing_verify`], and `key_data` must be null or valid for
/// reading `key_data_size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_signing_verify_with_public_key(
    data: *const c_void,
    data_size: usize,
    signature: *const c_char,
    signature_size: usize,
    key_data: *const c_void,
    key_data_size: usize,
) -> SigningResult {
    if data.is_null() || signature.is_null() || key_data.is_null() {
        return TOBII_SIGNING_NULL_ARGUMENT;
    }
    // SAFETY: non-null, and the caller guarantees the extents.
    let (bytes, sig, key) = unsafe {
        (
            slice::from_raw_parts(data.cast::<u8>(), data_size),
            slice::from_raw_parts(signature.cast::<u8>(), signature_size),
            slice::from_raw_parts(key_data.cast::<u8>(), key_data_size),
        )
    };
    match pem_from_memory(key) {
        Pem::Failed(result) => result,
        Pem::Text(pem) => verify_with(&pem, bytes, sig, TOBII_SIGNING_VERIFY_PEM_FAILED),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::io::Write as _;

    use super::*;

    /// A throwaway RSA-2048 pair, generated for these tests and used for
    /// nothing else: signing has no device behind it, so a real key would
    /// buy nothing and committing one would be worse.
    const PRIVATE_PEM: &str = include_str!("testdata/signing-test-private.pem");
    const PUBLIC_PEM: &str = include_str!("testdata/signing-test-public.pem");

    /// The enum is contiguous 0..=16, as the DLL's is, and every value of it
    /// is also a valid `tobii_error_t` — which is exactly why it must not be
    /// mapped through one.
    #[test]
    fn the_result_enum_is_contiguous_and_collides_with_tobii_error_t() {
        let all = [
            TOBII_SIGNING_OK,
            TOBII_SIGNING_KEY_BUFFER_FAILED,
            TOBII_SIGNING_NULL_ARGUMENT,
            TOBII_SIGNING_SIGN_OPEN_FAILED,
            TOBII_SIGNING_SIGN_PEM_FAILED,
            TOBII_SIGNING_VERIFY_OPEN_FAILED,
            TOBII_SIGNING_VERIFY_PEM_FAILED,
            TOBII_SIGNING_DIGEST_CONTEXT_FAILED,
            TOBII_SIGNING_DIGEST_FAILED,
            TOBII_SIGNING_SIGN_INIT_FAILED,
            TOBII_SIGNING_SIGN_UPDATE_FAILED,
            TOBII_SIGNING_SIGN_FINAL_FAILED,
            TOBII_SIGNING_ALLOCATION_FAILED,
            TOBII_SIGNING_BASE64_FAILED,
            TOBII_SIGNING_VERIFY_INIT_FAILED,
            TOBII_SIGNING_VERIFY_UPDATE_FAILED,
            TOBII_SIGNING_NOT_VERIFIED,
        ];
        assert_eq!(
            all,
            core::array::from_fn::<_, 17, _>(|i| {
                SigningResult::try_from(i).expect("0..17 fits a c_int")
            })
        );
        assert_eq!(
            TOBII_SIGNING_NOT_VERIFIED,
            crate::status::TOBII_ERROR_CALLBACK_IN_PROGRESS
        );
    }

    /// Sign with the key in memory, verify with the public key in memory:
    /// the signature is base64, NUL-terminated, and its reported length is
    /// the encoded length without the terminator.
    #[test]
    fn a_signature_made_from_memory_verifies_from_memory() {
        let data = b"the quick brown fox";
        let mut signature = [0i8; SIGNATURE_BUFFER];
        let mut size = 0usize;
        // SAFETY: every pointer is to a live local of the right extent.
        let signed = unsafe {
            tobii_signing_sign_with_private_key(
                data.as_ptr().cast(),
                data.len(),
                signature.as_mut_ptr().cast(),
                &raw mut size,
                PRIVATE_PEM.as_ptr().cast(),
                PRIVATE_PEM.len(),
            )
        };
        assert_eq!(signed, TOBII_SIGNING_OK);
        assert!(size > 0 && size < SIGNATURE_BUFFER, "size {size}");
        // SAFETY: `sign_with` wrote `size` bytes and a terminator.
        let text = unsafe { CStr::from_ptr(signature.as_ptr().cast()) };
        assert_eq!(text.to_bytes().len(), size, "NUL-terminated at the size");
        assert!(STANDARD.decode(text.to_bytes()).is_ok(), "base64");

        // SAFETY: as above.
        let verified = unsafe {
            tobii_signing_verify_with_public_key(
                data.as_ptr().cast(),
                data.len(),
                signature.as_ptr().cast(),
                size,
                PUBLIC_PEM.as_ptr().cast(),
                PUBLIC_PEM.len(),
            )
        };
        assert_eq!(verified, TOBII_SIGNING_OK);

        // Another message under the same signature does not verify.
        let other = b"the quick brown fix";
        // SAFETY: as above.
        let refused = unsafe {
            tobii_signing_verify_with_public_key(
                other.as_ptr().cast(),
                other.len(),
                signature.as_ptr().cast(),
                size,
                PUBLIC_PEM.as_ptr().cast(),
                PUBLIC_PEM.len(),
            )
        };
        assert_eq!(refused, TOBII_SIGNING_NOT_VERIFIED);
    }

    /// The file-path variants read the same keys from disk, and a path that
    /// cannot be opened is the open failure of the call that asked.
    #[test]
    fn the_file_variants_sign_and_verify_and_report_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("tobii-signing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let (sk, pk) = (dir.join("sk.pem"), dir.join("pk.pem"));
        for (path, pem) in [(&sk, PRIVATE_PEM), (&pk, PUBLIC_PEM)] {
            let mut f = std::fs::File::create(path).expect("write key");
            f.write_all(pem.as_bytes()).expect("write key");
        }
        let sk_c = CString::new(sk.to_str().expect("utf-8")).expect("no NUL");
        let pk_c = CString::new(pk.to_str().expect("utf-8")).expect("no NUL");
        let missing =
            CString::new(dir.join("absent.pem").to_str().expect("utf-8")).expect("no NUL");

        let data = b"signed from a file";
        let mut signature = [0i8; SIGNATURE_BUFFER];
        let mut size = 0usize;
        // SAFETY: every pointer is to a live local or CString of the right
        // extent.
        unsafe {
            assert_eq!(
                tobii_signing_sign(
                    data.as_ptr().cast(),
                    data.len(),
                    signature.as_mut_ptr().cast(),
                    &raw mut size,
                    sk_c.as_ptr()
                ),
                TOBII_SIGNING_OK
            );
            assert_eq!(
                tobii_signing_verify(
                    data.as_ptr().cast(),
                    data.len(),
                    signature.as_ptr().cast(),
                    size,
                    pk_c.as_ptr()
                ),
                TOBII_SIGNING_OK
            );
            assert_eq!(
                tobii_signing_sign(
                    data.as_ptr().cast(),
                    data.len(),
                    signature.as_mut_ptr().cast(),
                    &raw mut size,
                    missing.as_ptr()
                ),
                TOBII_SIGNING_SIGN_OPEN_FAILED
            );
            assert_eq!(
                tobii_signing_verify(
                    data.as_ptr().cast(),
                    data.len(),
                    signature.as_ptr().cast(),
                    size,
                    missing.as_ptr()
                ),
                TOBII_SIGNING_VERIFY_OPEN_FAILED
            );
        }
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    /// A null argument is answered before anything is read or written, where
    /// the DLL would dereference it.
    #[test]
    fn a_null_argument_is_refused_without_a_write() {
        let mut size = 123usize;
        let mut signature = [7i8; 4];
        let void: *const c_void = std::ptr::null();
        let text: *const c_char = std::ptr::null();
        // SAFETY: the calls read nothing; the nulls are the point.
        unsafe {
            assert_eq!(
                tobii_signing_sign(void, 0, signature.as_mut_ptr(), &raw mut size, text),
                TOBII_SIGNING_NULL_ARGUMENT
            );
            assert_eq!(
                tobii_signing_verify(void, 0, text, 0, text),
                TOBII_SIGNING_NULL_ARGUMENT
            );
            assert_eq!(
                tobii_signing_sign_with_private_key(
                    void,
                    0,
                    signature.as_mut_ptr(),
                    &raw mut size,
                    void,
                    0
                ),
                TOBII_SIGNING_NULL_ARGUMENT
            );
            assert_eq!(
                tobii_signing_verify_with_public_key(void, 0, text, 0, void, 0),
                TOBII_SIGNING_NULL_ARGUMENT
            );
        }
        assert_eq!(size, 123, "nothing written");
        assert_eq!(signature, [7i8; 4], "nothing written");
    }
}
