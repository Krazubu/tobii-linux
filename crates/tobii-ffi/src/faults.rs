//! The tracker's fault and warning lists: the six entry points 4.25 adds
//! over the two string states 4.1 already served.
//!
//! The lists are the same text `tobii_get_state_string` gives for
//! `TOBII_STATE_FAULT` and `TOBII_STATE_WARNING`: the tracker's legacy status
//! strings, which the daemon keeps and re-sends on change. In 4.25 the
//! dependency runs the other way — its `tobii_get_state_string`
//! (0x1800834f0) is a wrapper over these two getters (0x1800836a4 to faults,
//! 0x180083630 to warnings) — but the answer is the same text either way, so
//! here the getters forward and there is one implementation.
//!
//! The DLL's receiver copies at most 0x1ff bytes into the caller's buffer and
//! terminates it (0x1800776b0), so the buffer must hold 512 bytes; both
//! getters take it as [`StateString`], as `tobii_get_state_string` does.

use std::ffi::c_void;

use crate::api::tobii_get_state_string;
use crate::device::{Callbacks, Device};
use crate::status::Status;
use crate::streams::{subscribe, unsubscribe};
use crate::types::{StateString, StatusTextFn, TOBII_STATE_FAULT, TOBII_STATE_WARNING};

/// The tracker's fault list (the DLL's property 0x15), as
/// `tobii_get_state_string(device, TOBII_STATE_FAULT, faults)`: an empty
/// string when the tracker reports no fault.
///
/// # Safety
/// `device` must be null or a live handle from `tobii_device_create` that is
/// not destroyed before the call returns; `faults` must be null or valid for
/// writing 512 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_get_faults(device: *mut Device, faults: *mut StateString) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { tobii_get_state_string(device, TOBII_STATE_FAULT, faults) }
}

/// The tracker's warning list (the DLL's property 0x16), as
/// [`tobii_get_faults`] for `TOBII_STATE_WARNING`.
///
/// # Safety
/// As [`tobii_get_faults`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_get_warnings(
    device: *mut Device,
    warnings: *mut StateString,
) -> Status {
    // SAFETY: forwarded under the same contract.
    unsafe { tobii_get_state_string(device, TOBII_STATE_WARNING, warnings) }
}

/// Define the subscribe/unsubscribe pair of one status-text stream.
macro_rules! status_text_pair {
    ($(
        $(#[$doc:meta])*
        $sub:ident / $unsub:ident: $slot:ident;
    )+) => { $(
        $(#[$doc])*
        ///
        /// Each change delivers the whole list, as the getter writes it. The
        /// callback runs on the thread calling
        /// `tobii_device_process_callbacks`, under the contract in the
        /// `streams` module documentation; the string it is handed is valid
        /// for the call only.
        ///
        /// # Safety
        /// See the `streams` module documentation.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $sub(
            device: *mut Device,
            callback: Option<StatusTextFn>,
            user_data: *mut c_void,
        ) -> Status {
            // SAFETY: forwarded under the same contract.
            unsafe { subscribe(device, |c: &mut Callbacks| &mut c.$slot, callback, user_data) }
        }

        #[doc = concat!("Undo `", stringify!($sub), "`.")]
        ///
        /// # Safety
        /// `device` must be null or a live handle that is not destroyed
        /// before the call returns.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $unsub(device: *mut Device) -> Status {
            // SAFETY: forwarded under the same contract.
            unsafe { unsubscribe(device, |c: &mut Callbacks| &mut c.$slot) }
        }
    )+ };
}

status_text_pair! {
    /// The tracker's fault list on every change, from the daemon's
    /// `FAULTS_CHANGED` notification. Subscribing starts the daemon's
    /// notification stream, so a subscriber sees the list the tracker
    /// reports from then on, not the one standing at subscribe:
    /// [`tobii_get_faults`] answers that.
    tobii_faults_subscribe / tobii_faults_unsubscribe: faults;
    /// The tracker's warning list on every change, from `WARNINGS_CHANGED`,
    /// as the fault list above.
    tobii_warnings_subscribe / tobii_warnings_unsubscribe: warnings;
}
