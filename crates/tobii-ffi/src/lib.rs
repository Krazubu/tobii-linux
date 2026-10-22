//! `libtobii.so`: the C ABI of Tobii's Stream Engine 4.1, backed by the
//! `tobiid` daemon.
//!
//! Every one of the 153 entry points `tobii_stream_engine.dll` 4.1.0.3
//! exports is exported here with the same signature, and no other, so any
//! Stream Engine client links and runs. What stands behind each:
//!
//! - **Implemented** — device lifetime and callbacks, device info (the public
//!   and the internal one), track box,
//!   display area and mounting (read and write), states, capabilities, the
//!   gaze point, gaze origin, eye position, user position guide, presence,
//!   head pose, gaze data, raw gaze (the Stream Engine's own record of each
//!   gaze frame), the foveated rendering gaze point (made from raw gaze as
//!   the DLL makes it), IR image and notification streams, the custom stream
//!   (the tracker's stream 0x1770 as custom stream 0, subscribed by the
//!   DLL's rules; its frames carry no data on the ET5), image collection's
//!   subscribe (the tracker's stream 0x508, started as the DLL starts it;
//!   the ET5 refuses it, so the subscribe answers `TOBII_ERROR_INTERNAL`,
//!   as the DLL's does, and no frame is ever delivered), 2-D calibration (a
//!   session's result is saved as the user's calibration only when its owner
//!   calls `tobii_calibration_stop`) and discarding a 2-D point, the
//!   device/host clock pair (`tobii_timesync`), the tracker's stream
//!   catalogue and its streams' columns, internal-stream support (the IR
//!   image, and the custom stream and image collection while that catalogue
//!   lists them, as the DLL answers for an ET5), device pause and resume,
//!   power save (entered and left, and its state and remote wake's), the
//!   tracker's extension lists (empty on an ET5; see `state_commands`), and
//!   the device name (kept by the host, not the tracker). The hardware
//!   configuration is implemented provisionally; the ET5 has reported none
//!   on Linux, so it answers `TOBII_ERROR_NOT_SUPPORTED` there.
//! - **Answered locally** — API version (4.1.0.3), error texts (the DLL's),
//!   system clock, output frequency (33 Hz), enabled eye (both), feature
//!   group (consumer), license validation (every key valid), display-area
//!   calculation, calibration parsing, internal-capability support (eyeball
//!   centres only), lens configuration writability (never) and usage
//!   statistics (`TOBII_ERROR_NOT_AVAILABLE`, as the DLL answers for a
//!   tracker it drives itself while Tobii's Stream Engine Services do not
//!   run), setting the illumination mode and enabling an extension
//!   (`TOBII_ERROR_INSUFFICIENT_LICENSE`, as the DLL answers a consumer ET5
//!   without asking the tracker).
//! - **`TOBII_ERROR_NOT_SUPPORTED`** — what the ET5 refuses, remote wake and
//!   reading the illumination mode (see `state_commands`), the configuration
//!   key, which it refuses until realm 10000 is open (where the DLL answers a
//!   consumer `TOBII_ERROR_INSUFFICIENT_LICENSE`), and everything it was
//!   never observed doing: wearable, face id, firmware, diagnostics, custom
//!   commands, licence storage on the device, 3-D and per-eye calibration,
//!   the calibration stimulus points, and the subscribes and unsubscribes of
//!   internal streams 3, 4, 5, 7 and 8: low-frequency head rotation and
//!   position, multiple faces position, wearable limited image and secondary
//!   camera image. For the wearable entry points, the digital sync port, face
//!   id, 3-D and per-eye calibration, the stimulus points and those five
//!   streams that is the DLL's own answer for a tracker it drives itself
//!   through its in-process tracker module, once their arguments check out
//!   (see `wearable`, `advanced`, `calibration`,
//!   `tobii_calibration_stimulus_points_get`, `internal` and
//!   `stub::refused`); per-eye calibration answers it only inside this
//!   handle's own calibration session, and
//!   `TOBII_ERROR_CALIBRATION_NOT_STARTED` outside it, as the DLL does. For
//!   the face types, which the DLL asks the tracker for, and licence storage,
//!   which it offers a tracker with persistent storage, it is inferred (see
//!   `advanced` and `licensing`).
//!
//! Nothing is gated by a licence but two writes. What the Stream Engine
//! reserves for a higher feature group or an extra licence (gaze data,
//! timesync, calibration, display-area and name writes, the IR image, raw
//! gaze, the internal device info, the stream catalogue and its streams'
//! columns, pause) works here under the consumer group this library reports
//! (see `licensing`). Where the DLL answers
//! `TOBII_ERROR_INSUFFICIENT_LICENSE` to that group for what the ET5 cannot
//! do anyway (per-eye calibration inside a session, setting the face type),
//! libtobii answers what the DLL would past that check (see
//! `tobii_calibration_collect_data_per_eye_2d` and `tobii_set_face_type`);
//! it asks the tracker for the extension lists past that check too. Setting
//! the illumination mode and enabling an extension keep the DLL's
//! `TOBII_ERROR_INSUFFICIENT_LICENSE`, by choice: nothing could switch an
//! extension off again (see `state_commands`).
//!
//! A call from inside a callback is `TOBII_ERROR_CALLBACK_IN_PROGRESS`, as in
//! the Stream Engine, from every implemented entry point that takes a device
//! handle, `tobii_device_destroy` included, and from the entry points that
//! answer as the DLL does for an ET5 once their arguments check out:
//! `tobii_calibration_stimulus_points_get`, the refused internal streams'
//! subscribes and unsubscribes, those answered through `stub::refused`, the
//! per-eye calibration calls, `tobii_send_statistics`,
//! `tobii_set_illumination_mode` and `tobii_enable_extension`, but not
//! `tobii_set_display_info` or `tobii_set_fw_upgrade_allowed`, in which the
//! DLL checks no callback flag. Four more refuse it once their arguments
//! check out, as in the DLL: `tobii_api_destroy` (the DLL keeps its callback
//! flag in the API instance), `tobii_device_create` and
//! `tobii_device_create_ex` (a callback could not destroy what they make) and
//! `tobii_calibration_parse`. `tobii_calibration_retrieve`'s receiver counts
//! as a callback, as in the DLL, which sets its callback flag around it:
//! every call from inside it that a callback could not make either is
//! refused, though as it runs with no lock held it may wait for other
//! threads' calls.
//!
//! That refusal covers only the thread the callback runs on (the flag is
//! thread-local); other threads' calls go on. Threads may share a device, as
//! the Stream Engine promises thread safety across all its functions: a
//! device's state is locked by concern, as the DLL's is (see
//! `device::Device`), so requests and subscription changes on one device run
//! one at a time, in the order they are called, its callbacks run one at a
//! time on whichever thread processes it, and a process call made while
//! another thread dispatches it returns at once. `tobii_device_destroy` and
//! `tobii_api_destroy` take no lock, as in the DLL: no other thread may be
//! inside a call on the handle, or use it afterwards. A callback (not
//! `tobii_calibration_retrieve`'s receiver, which holds no lock), or the
//! logger, must not block on another thread's call into any device (nor on a
//! thread that waits for one), which can deadlock, as in the Stream Engine:
//! only `tobii_device_process_callbacks` and `tobii_wait_for_callbacks` are
//! sure to return while a callback runs; any other call on its device may
//! wait for it, itself or queued behind one that does, and a call on another
//! device may wait for that device's own callback, which may be waiting in
//! turn. Where the DLL differs: its API mutex, a critical section, promises
//! no order among its waiters (Windows semantics, not read from the DLL),
//! where libtobii's lets requests and subscription changes in in the order
//! they are called; its wait skips a device another thread is processing,
//! returning at once when it was the only one, where libtobii's waits as for
//! any other; its subscribe, but for the custom stream's, holds the device's
//! callbacks back for its round trip, where libtobii's lets them run; its
//! `tobii_calibration_retrieve` calls the receiver under the device's API
//! mutex, where libtobii's holds no lock; and many of its calls log their
//! error lines under that mutex, where libtobii logs with none of the
//! logging call's locks held (see `logger`).
//!
//! An entry point that takes a device handle checks the callback first, then
//! a null device, then its other arguments (`TOBII_ERROR_INVALID_PARAMETER`
//! both). Three check arguments first: `tobii_calibration_start` its eye,
//! `tobii_calibration_apply` its blob and `tobii_wait_for_callbacks` its
//! count and null handles. The DLL checks the handle, and sometimes more,
//! first, so the two differ only for an invalid argument passed from inside a
//! callback. Two check no callback, as in the DLL: `tobii_set_display_info`,
//! which checks its pointers only, and `tobii_set_fw_upgrade_allowed`, which
//! checks nothing. The DLL also refuses only calls into the API instance
//! whose callbacks run, where here any is refused, and it counts the receiver
//! of `tobii_enumerate_local_device_urls(_ex)` as a callback, where here its
//! `# Safety` section forbids re-entry.
//!
//! `tobii_calibration_parse` checks in the DLL's order: a null `api` or
//! `data`, a `data_size` under 8 or a null `receiver`
//! (`TOBII_ERROR_INVALID_PARAMETER`), then the callback, then the whole blob
//! before it hands out a point. Data that is not a valid calibration
//! (whatever `tobii_calib::blob::points` refuses, a blob shorter than its
//! 44-byte header included) is `TOBII_ERROR_OPERATION_FAILED`, as the 4.1
//! docs say. The DLL returns that only for a negative point count and reads
//! the rest as given, past `data_size` if the blob says so, with
//! `TOBII_ERROR_NO_ERROR`.
//!
//! `tobii_device_create` connects to the daemon (auto-spawning it if needed),
//! so several processes can use the device at once. Threads that create
//! devices at once while no daemon runs spawn one between them: the others
//! wait for that spawn, then connect to its daemon or fail as it did (two
//! processes doing so can still spawn one each). Pump with
//! `tobii_wait_for_callbacks` + `tobii_device_process_callbacks` exactly like
//! the Stream Engine. Every sample's timestamp is on the host clock
//! `tobii_system_clock` reads, as the daemon sends it (gaze data's tracker
//! time and raw gaze's aside; the foveated rendering gaze point takes the
//! host time tobiid sends beside raw gaze's, or raw gaze's own from a tobiid
//! from before that): libtobii passes the daemon's values through. The
//! daemon's engine maps the tracker's clock onto the host's by the smallest
//! receipt-minus-tracker time of the gaze frames and IR images of the last
//! 120 s, started afresh at every tracker init (which may restart the
//! tracker's clock), so it follows the drift and every client gets the same
//! stamps. Tracker time is left only in gaze data's and raw gaze's
//! `timestamp_tracker_us` and `tobii_timesync`'s `tracker_us`, and
//! `tobii_update_timesync` has no offset to refresh.
//!
//! When the daemon connection is lost (tobiid stopped, crashed or was
//! restarted, or dropped a client that stopped reading),
//! `tobii_device_process_callbacks` delivers what had arrived and then
//! returns `TOBII_ERROR_CONNECTION_FAILED` on every call until
//! `tobii_device_reconnect` connects again. libtobii never reconnects by
//! itself, so an application that never calls it stays lost until it creates
//! the device again. `tobii_wait_for_callbacks` wakes for the loss until a
//! process call reports it, so a wait-and-process loop wakes once, and then
//! waits out its timeout as for a quiet device; as in the DLL it never
//! returns `TOBII_ERROR_CONNECTION_FAILED`. A reconnect only connects: unlike
//! `tobii_device_create` it never spawns a daemon, and any failure is
//! `TOBII_ERROR_CONNECTION_FAILED` within ~500 ms (at once when nothing
//! listens). On a device shared between threads that counts from when the
//! requests, subscription changes and other reconnects other threads have
//! under way or queued ahead of it, and then another thread's dispatch, have
//! finished: the reconnect waits for them before its round trip, and until
//! it has swapped connections or failed a process call on another thread
//! returns at once, delivering nothing, so no sample tobiid sends both
//! connections is delivered twice. Of what the old connection brought and
//! was not delivered, the samples are dropped, so a reconnect of a live
//! connection may lose those tobiid sent it alone, but the notifications are
//! kept, ahead of the new connection's (tobiid sends each once), and one sent
//! to both may be delivered again, after later ones: the last delivered is
//! current.
//! It restores the subscriptions and the holds of the custom stream and
//! image collection (tobiid starts the streams again itself), not a
//! calibration session or pause the lost connection held.
//!
//! `TOBII_ERROR_CONNECTION_FAILED` has a second source: a request that needs
//! the tracker live (a clock pair, a pause, a calibration, a display-area
//! write, the start of the custom stream or image collection) gets it from a
//! daemon that has no tracker, over a connection that is fine. Not so a
//! start of a stream tobiid holds already (for another connection, say),
//! which joins it, `TOBII_ERROR_NO_ERROR` with or without the tracker, nor
//! one whose tracker tobiid loses while the start waits on it, which gets
//! the tracker result the DLL's map answers for that
//! (`TOBII_ERROR_CONNECTION_FAILED_DRIVER` for the custom stream,
//! `TOBII_ERROR_NO_ERROR` for image collection). While the tracker is away,
//! a reconnect succeeds without bringing it back. A tracker unplugged while
//! the daemon runs is not a lost connection and never surfaces through
//! `tobii_device_process_callbacks`: its samples stop, and resume on the
//! same connection once the daemon has started the tracker again after a
//! replug. The next process call tells the two apart:
//! a request that failed because the connection is gone has marked it lost,
//! so that call reports the loss, while after a daemon's answer that it has
//! no tracker it returns `TOBII_ERROR_NO_ERROR`.
//!
//! A request waits for tobiid's answer longer than tobiid's own deadlines
//! let it take (see `timeouts`), so that `TOBII_ERROR_TIMED_OUT` from one is
//! tobiid's answer (or a tobiid held up past them), not libtobii giving up
//! on a request tobiid then carries out, but for one made right after a
//! stop of the custom stream or image collection that libtobii sent without
//! waiting (after tobiid refused the unsubscribe's stop, or after libtobii
//! gave up on a subscribe's start): tobiid runs that stop first, for up to
//! 53 s, which the request's timeout does not count. A call answered from
//! what the tracker reported waits at most ~12 s, for its first init. One
//! that needs the tracker live waits for it to stream (~12 s from cold) and
//! answer, and at worst for tobiid's own deadlines: 30 s for the tracker to
//! take each command, plus the command's (up to 35 s for
//! `tobii_calibration_retrieve`, about a minute for a subscribe or
//! unsubscribe of the custom stream or image collection, which may also
//! wait 20 s for another start or stop of either stream, about 3 min for a
//! `tobii_calibration_start` that fails and puts the calibration back).
//!
//! libtobii prints nothing, and nothing outside it can subscribe to its
//! `tracing` events (it is a cdylib with its own copy of `tracing`): the
//! `tobii_custom_log_t` an application hands `tobii_api_create` is the only
//! way to see its diagnostics. They are its own, and few: ERROR for a refused
//! `field_of_use`, a failed connect or reconnect, a lost connection when
//! `tobii_device_process_callbacks` reports it (once per loss), a daemon
//! reply that does not decode and a `tobii_calibration_stop` that failed
//! after the daemon saved the calibration, WARN, once per device, for a head
//! pose the daemon does not send and for a subscribe of the custom stream or
//! image collection it refuses as not supported though the tracker's
//! catalogue lists the stream (a daemon older than libtobii does both),
//! INFO for each connect and reconnect. The logger is called on the thread
//! inside the `tobii_*` call that logs, with no lock of that call's held and
//! under the callback guard, so a call from inside it, on that thread, that
//! a callback could not make either is `TOBII_ERROR_CALLBACK_IN_PROGRESS`;
//! threads may log at once, and their lines may interleave. A
//! `tobii_custom_alloc_t` is checked as in the DLL and never called (see
//! `logger` and `tobii_api_create`).
//!
//! Every entry point takes raw handles from C, so each is an `unsafe fn`
//! whose `# Safety` section states what the caller must uphold; the `unsafe`
//! blocks inside are kept to the single pointer operation that needs them.
//! Entry points without an implementation are safe functions, but for the
//! per-eye calibration calls, which borrow the device to read its calibration
//! session flag: the stubs read none of their arguments (see `stub`), and
//! `tobii_calibration_stimulus_points_get`, the refused internal streams'
//! subscribes and unsubscribes, the entry points answered through
//! `stub::refused` and `tobii_set_display_info` only compare their pointers
//! with null, and `tobii_set_fw_upgrade_allowed` reads nothing.

mod advanced;
mod api;
mod calibration;
mod config;
mod device;
mod diagnostics;
mod display_id;
mod gaze_hid;
mod internal;
mod licensing;
mod logger;
mod state_commands;
mod status;
mod streams;
mod stub;
mod timeouts;
mod types;
mod wearable;

pub use device::{Api, Device};
pub use status::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use crate::status::{Status, TOBII_ERROR_NOT_SUPPORTED};
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    /// Every stub answers `TOBII_ERROR_NOT_SUPPORTED` whatever it is given,
    /// and naming each one here catches a typo before `make verify-abi` does.
    #[test]
    fn stubs_are_not_supported() {
        let n: *mut c_void = null_mut();
        let c: *const c_void = null();
        let results: Vec<(&str, Status)> = vec![
            // `open_realm` is no longer here: it answers an ET5
            // `TOBII_ERROR_INSUFFICIENT_LICENSE` past its arguments and
            // `TOBII_ERROR_INVALID_PARAMETER` for a null device, so it has
            // its own tests in `gaze_hid`.
            (
                "send_custom_command",
                crate::internal::tobii_send_custom_command(n, 0, c, 0, c, n),
            ),
        ];
        for (name, status) in results {
            assert_eq!(status, TOBII_ERROR_NOT_SUPPORTED, "{name}");
        }
    }
}
