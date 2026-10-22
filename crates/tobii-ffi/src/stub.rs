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
    Status, TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_INSUFFICIENT_LICENSE,
    TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NOT_SUPPORTED,
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

/// The answer of `export`, an entry point the DLL refuses for an ET5 on its
/// licence: `TOBII_ERROR_CALLBACK_IN_PROGRESS` from inside a callback, then
/// `TOBII_ERROR_INVALID_PARAMETER` for a null device or when an argument the
/// DLL requires is null (`missing`), and otherwise
/// `TOBII_ERROR_INSUFFICIENT_LICENSE`, with nothing written and no daemon
/// round trip.
///
/// The DLL gets there without asking the tracker: `device_create` gives a
/// consumer device module licence level 0, while these modules want more
/// before they send anything. The level stored at 0x180158844 is `r10d`,
/// which the switch on the feature group `[dev+0x9790]` at 0x180158771 sets
/// (3 to 1, 4 to 2, 5 to 3) and otherwise leaves at its default of zero, as
/// for a consumer's group 2; 0x180172800 writes the field too but has no
/// caller anywhere in the image. A consumer ET5 is level 0 because
/// `commands_device_connect` writes feature group 2 by default
/// (0x180032b95) and only raises it for a device class above 0, which a
/// `Peripheral` whose serial is none of the eleven professional prefixes is
/// not (0x18016be5d..0x18016bec8).
///
/// As with [`refused`], the DLL checks its nulls before the callback flag and
/// this crate checks the flag first, so the two differ only for a null
/// argument passed from inside a callback. The device is only compared with
/// null, never borrowed.
pub(crate) fn licence_refused(export: &'static str, device: *mut Device, missing: bool) -> Status {
    if in_callback() {
        return TOBII_ERROR_CALLBACK_IN_PROGRESS;
    }
    if device.is_null() || missing {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    tracing::trace!(
        export,
        "insufficient licence, as the DLL for a consumer ET5"
    );
    TOBII_ERROR_INSUFFICIENT_LICENSE
}
