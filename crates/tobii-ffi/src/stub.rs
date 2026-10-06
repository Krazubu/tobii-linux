//! Entry points that exist so every Stream Engine client links, but have no
//! implementation behind them, and the answer of those the DLL refuses for an
//! ET5 once their arguments check out.
//!
//! A stub is a safe `extern "C"` function: it reads none of its arguments, so
//! nothing a caller passes can make it unsound, and it needs no `# Safety`
//! section. Its parameters are typed with "any bit pattern is valid" types
//! (`*const c_void`, integers, `f32`) whatever the C header says, because the
//! C types only matter to a callee that reads them. An entry point answered
//! through [`refused`] stays as safe: it only compares its pointers with null.

use crate::device::{Device, in_callback};
use crate::status::{
    Status, TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_INVALID_PARAMETER,
    TOBII_ERROR_NOT_SUPPORTED,
};

/// Define `extern "C"` stubs returning `TOBII_ERROR_NOT_SUPPORTED`.
macro_rules! not_supported {
    ($( $(#[$m:meta])* fn $name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ); )+) => { $(
        $(#[$m])*
        #[doc = concat!(
            "`", stringify!($name), "`: not implemented by libtobii.so; returns ",
            "`TOBII_ERROR_NOT_SUPPORTED` without reading its arguments."
        )]
        #[unsafe(no_mangle)]
        pub extern "C" fn $name($( $arg: $ty ),*) -> $crate::status::Status {
            $( let _ = $arg; )*
            tracing::trace!(concat!(stringify!($name), ": not supported"));
            $crate::status::TOBII_ERROR_NOT_SUPPORTED
        }
    )+ };
}

pub(crate) use not_supported;

/// The answer of `export`, an entry point the DLL refuses for an ET5 whatever
/// the device's state: `TOBII_ERROR_CALLBACK_IN_PROGRESS` from inside a
/// callback, then `TOBII_ERROR_INVALID_PARAMETER` for a null device or when
/// an argument the DLL requires is null (`missing`), and otherwise
/// `TOBII_ERROR_NOT_SUPPORTED`, with nothing written and no daemon round
/// trip, keeping a `tracing::trace!` naming the export.
///
/// The DLL checks its nulls before the callback flag; libtobii refuses a call
/// inside a callback first, as for every entry point that takes a handle, so
/// the two differ only for a null argument passed from inside a callback.
/// Unlike the DLL, which logs most refusals at ERROR, nothing reaches the
/// application's logger. The device is only compared with null, never
/// borrowed.
pub(crate) fn refused(export: &'static str, device: *mut Device, missing: bool) -> Status {
    if in_callback() {
        return TOBII_ERROR_CALLBACK_IN_PROGRESS;
    }
    if device.is_null() || missing {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    tracing::trace!(export, "not supported");
    TOBII_ERROR_NOT_SUPPORTED
}
