//! `tobii_licensing.h`. There is nothing to license on Linux: every key
//! validates, and the feature group is consumer. No entry point checks it.
//! What the Stream Engine reserves for a higher group is served anyway: gaze
//! data and `tobii_timesync` (professional), calibration and the
//! display-area and device-name writes (config or professional), the IR
//! image (additional features), raw gaze, the internal device info and the
//! stream catalogue (internal) and, in the DLL's own service, pause
//! (internal). Storing and
//! retrieving keys on the device answer `TOBII_ERROR_NOT_SUPPORTED` once
//! their arguments check out, as the DLL does for a tracker without
//! persistent storage (inferred for the ET5; see `tobii_license_key_store`).

use std::ffi::{c_char, c_void};

use crate::api::create_device;
use crate::device::{Api, Device, device_ref};
use crate::status::{Status, TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NO_ERROR};
use crate::stub::refused;
use crate::types::{
    FieldOfUse, LicenseKey, TOBII_FEATURE_GROUP_CONSUMER, TOBII_LICENSE_VALIDATION_RESULT_OK,
};

/// `tobii_device_create` with license keys: each key is reported valid
/// without being read.
///
/// # Safety
/// As `tobii_device_create`; `license_results` must be null or valid for
/// writing `license_count` `tobii_license_validation_result_t` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_device_create_ex(
    api: *mut Api,
    _url: *const c_char,
    field_of_use: FieldOfUse,
    license_keys: *const LicenseKey,
    license_count: i32,
    license_results: *mut u32,
    device: *mut *mut Device,
) -> Status {
    let Ok(count) = usize::try_from(license_count) else {
        return TOBII_ERROR_INVALID_PARAMETER;
    };
    if count > 0 && license_keys.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    // SAFETY: forwarded under the same contract.
    let status = unsafe { create_device(api, field_of_use, device) };
    if status == TOBII_ERROR_NO_ERROR && !license_results.is_null() {
        for i in 0..count {
            // SAFETY: the caller guarantees `license_count` writable results.
            unsafe {
                license_results
                    .add(i)
                    .write(TOBII_LICENSE_VALIDATION_RESULT_OK);
            }
        }
    }
    status
}

/// Consumer.
///
/// # Safety
/// `device` must be null or a live handle that is not destroyed before the
/// call returns; `feature_group` must be null or valid for writing one
/// `tobii_feature_group_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_get_feature_group(
    device: *mut Device,
    feature_group: *mut u32,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    if let Err(status) = unsafe { device_ref(device) } {
        return status;
    }
    if feature_group.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    // SAFETY: non-null, and the caller guarantees it is writable.
    unsafe { feature_group.write(TOBII_FEATURE_GROUP_CONSUMER) };
    TOBII_ERROR_NO_ERROR
}

/// Store a licence key on the device: `TOBII_ERROR_NOT_SUPPORTED` once the
/// device checks out, with `data` and `size` unread (inferred).
///
/// The DLL (0x1801505c0) checks a null device, then the callback flag, then
/// answers `TOBII_ERROR_NOT_SUPPORTED` (0x18015069c) unless the device's
/// command list (`[dev+0x88c0]`) holds PRP command 0x13; it never checks
/// `data` or `size`. Its own tracker module lists 0x13 and 0x14 (its
/// commands 0xc and 0xd, mapped at 0x18001edc5 and 0x18001edcc) only when the
/// tracker answers its persistent-file enumeration (command 3060,
/// 0x1801a0390) at connect (0x18016bcc8), and then stores the key on the
/// tracker. No capture holds 3060. `TOBII_CAPABILITY_PERSISTENT_STORAGE`,
/// which the DLL derives from the same list (0x180141d65), is unsupported
/// here, and the daemon keeps nothing on the tracker, so the answer is the
/// DLL's for a tracker without that storage. The pointer is never read, so
/// the function is safe.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_license_key_store(
    device: *mut Device,
    _data: *mut c_void,
    _size: usize,
) -> Status {
    refused("tobii_license_key_store", device, false)
}

/// Read back the licence keys stored on the device:
/// `TOBII_ERROR_NOT_SUPPORTED` once the device and receiver check out, with
/// the receiver never called (inferred, as `tobii_license_key_store`). The
/// DLL (0x1801502e0) checks a null device, then a null receiver, then the
/// callback flag, then answers `TOBII_ERROR_NOT_SUPPORTED` (0x18015042b)
/// unless the command list holds PRP command 0x14.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_license_key_retrieve(
    device: *mut Device,
    receiver: *const c_void,
    _user_data: *mut c_void,
) -> Status {
    refused("tobii_license_key_retrieve", device, receiver.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::{counting_device, destroy_untouched};
    use crate::status::{TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_NOT_SUPPORTED};
    use std::ptr;

    /// A receiver the refused retrieve is given and never calls.
    fn never_called() -> *const c_void {
        ptr::NonNull::<c_void>::dangling().as_ptr().cast_const()
    }

    /// The daemon counts every frame: none reaches it. The store reads
    /// neither its data nor its size, as in the DLL.
    #[test]
    fn licence_storage_is_not_supported_without_asking_the_daemon() {
        let (d, frames) = counting_device();
        let (refused, invalid) = (TOBII_ERROR_NOT_SUPPORTED, TOBII_ERROR_INVALID_PARAMETER);
        let n = ptr::null_mut();
        let mut key = [0x4b_u8; 16];
        let data = key.as_mut_ptr().cast::<c_void>();
        let got = [
            tobii_license_key_store(d, data, key.len()),
            tobii_license_key_store(d, n, 0),
            tobii_license_key_store(d, n, usize::MAX),
            tobii_license_key_store(n.cast(), data, key.len()),
            tobii_license_key_retrieve(d, never_called(), n),
            tobii_license_key_retrieve(d, ptr::null(), n),
            tobii_license_key_retrieve(n.cast(), never_called(), n),
        ];
        assert_eq!(
            got,
            [
                refused, refused, refused, invalid, refused, invalid, invalid
            ]
        );
        assert_eq!(key, [0x4b; 16], "nothing written");
        destroy_untouched(d, &frames);
    }

    /// Inside a callback both are refused before their arguments are read;
    /// the DLL checks the nulls first.
    #[test]
    fn licence_storage_is_refused_inside_a_callback() {
        let (d, frames) = counting_device();
        let n = ptr::null_mut();
        let mut got = Vec::new();
        crate::device::call(|| {
            got.extend([
                tobii_license_key_store(d, n, 0),
                tobii_license_key_store(n.cast(), n, 0),
                tobii_license_key_retrieve(d, never_called(), n),
                tobii_license_key_retrieve(n.cast(), ptr::null(), n),
            ]);
        });
        assert_eq!(got, [TOBII_ERROR_CALLBACK_IN_PROGRESS; 4]);
        destroy_untouched(d, &frames);
    }
}
