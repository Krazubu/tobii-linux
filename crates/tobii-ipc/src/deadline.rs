//! How long the daemon may take over a request: the deadlines on its way to
//! an answer, which a client must wait out rather than give up first and
//! leave the daemon carrying the request out (the clients' timeouts, in
//! [`timeout`](crate::timeout), are checked against these at compile time).
//!
//! A request answered from what the device reported at its last init waits
//! at most [`FACTS`] for the first one. A request that needs the device live
//! runs device commands on the daemon's engine, which takes a command only
//! once the device streams: each may wait [`QUEUE_ALLOWANCE`] for the engine
//! to take and write it, then its own deadline for the answer, [`command`]
//! in all. A request that runs several waits for each in turn
//! ([`commands`]). A clock pair waits for a gaze frame instead
//! ([`TIMESYNC`]), a pause or resume first for another to finish
//! ([`PAUSE_LOCK`]), and a catalogue stream's start or stop first for
//! another start or stop of one ([`STREAM_LOCK`]). What that comes to for
//! each request is in [`worst`], which the daemon's tests hold its requests'
//! device commands to.

use std::time::Duration;

/// How long a device command may wait for the engine to reach it (queued
/// behind other clients' commands, or the device still initialising: a cold
/// start takes ~12 s before the stream arms, and so may a re-open after a
/// stall) and to write it (a tracker starting its sensor refuses writes for
/// up to about 3 s; its sensor start takes 3.6 s) before its own deadline
/// starts.
pub const QUEUE_ALLOWANCE: Duration = Duration::from_secs(30);

/// How long a request waits for the device's init to report its facts.
pub const FACTS: Duration = Duration::from_secs(10);

/// How long a TIMESYNC waits for a gaze frame newer than the request. A cold
/// engine reports its facts within seconds but streams gaze only after its
/// second init (about 12 s on the ET5); this also covers a third.
pub const TIMESYNC: Duration = Duration::from_secs(25);

/// A calibration command the device answers at once (and collect, which
/// takes ~0.9 s).
pub const CALIBRATION_QUICK: Duration = Duration::from_secs(5);

/// Compute (~2 s) and uploading a ~660 KB calibration.
pub const CALIBRATION_SLOW: Duration = Duration::from_secs(10);

/// How long the device may take to acknowledge a display area.
pub const DISPLAY_AREA: Duration = Duration::from_secs(5);

/// How long the device may take to answer a pause or resume (3100). The DLL
/// allows 3 s; the init's resume has taken 3.5 s on Linux.
pub const PAUSE: Duration = Duration::from_secs(6);

/// How long a pause or resume waits for another to finish.
pub const PAUSE_LOCK: Duration = Duration::from_secs(20);

/// How long the device may take to answer a power-save command (3000,
/// 3010). The DLL allows 3 s (0x2dc6c0 at 0x18019d872 and 0x18019fd02), as
/// for the pause, which has taken longer on Linux; the ET5 answered at once
/// (probe 2026-10-21).
pub const POWER_SAVE: Duration = Duration::from_secs(6);

/// How long a power-save request waits for another to finish.
pub const POWER_SAVE_LOCK: Duration = Duration::from_secs(20);

/// How long the device may take to answer an extension list (1810, 1820):
/// the DLL allows 3 s (0x18019f5b0, 0x18019d050).
pub const EXTENSIONS: Duration = Duration::from_secs(6);

/// How long the device may take to answer a catalogue stream's start or
/// stop (1220, 1230): as long as the DLL's tracker module waits for the
/// answer to either (0x2dc6c0 µs, at 0x180160a1b for the custom stream's
/// 1220 and at 0x1801606f6 for its 1230). The ET5 answers within
/// milliseconds.
pub const STREAM: Duration = Duration::from_secs(3);

/// How long a catalogue stream's start or stop waits for another start or
/// stop of one to finish.
pub const STREAM_LOCK: Duration = Duration::from_secs(20);

/// The longest a device command whose answer is due within `deadline` keeps
/// a request waiting: the [`QUEUE_ALLOWANCE`] and then `deadline`.
#[must_use]
pub const fn command(deadline: Duration) -> Duration {
    QUEUE_ALLOWANCE.saturating_add(deadline)
}

/// The longest device commands whose answers are due within `deadlines`
/// keep a request that runs them one after another waiting: [`command`] of
/// each, added up.
#[must_use]
pub const fn commands(deadlines: &[Duration]) -> Duration {
    let mut sum = Duration::ZERO;
    let mut i = 0;
    while i < deadlines.len() {
        sum = sum.saturating_add(command(deadlines[i]));
        i += 1;
    }
    sum
}

/// The longer of two deadlines.
const fn longer(a: Duration, b: Duration) -> Duration {
    if a.as_nanos() >= b.as_nanos() { a } else { b }
}

/// How long the daemon may take over each request that waits for the
/// device, at worst: along the request's longest path, what it waits for
/// and the device commands it runs one after another. A client waits
/// these out, and the daemon's tests check that the commands a request
/// runs fit in its own.
pub mod worst {
    use std::time::Duration;

    use super::{
        CALIBRATION_QUICK, CALIBRATION_SLOW, DISPLAY_AREA, PAUSE, PAUSE_LOCK, POWER_SAVE_LOCK,
        STREAM, STREAM_LOCK, command, commands, longer,
    };

    /// A read of the facts: the device's first init.
    pub const FACTS: Duration = super::FACTS;

    /// A clock pair: a gaze frame newer than the request.
    pub const TIMESYNC: Duration = super::TIMESYNC;

    /// A pause or resume: another to finish, then 3100.
    pub const DEVICE_PAUSE: Duration = PAUSE_LOCK.saturating_add(command(PAUSE));

    /// Power save in or out: another to finish, then 3000 or 3010.
    pub const POWER_SAVE: Duration = POWER_SAVE_LOCK.saturating_add(command(super::POWER_SAVE));

    /// An extension list: one 1810 or 1820, as the DLL sends on every call.
    pub const EXTENSIONS: Duration = command(super::EXTENSIONS);

    /// A catalogue stream's start: the facts, another start or stop of a
    /// catalogue stream to finish, then 1220. The 1230 the daemon queues
    /// after a start the tracker may have taken unseen is not waited for.
    pub const CATALOGUE_STREAM_START: Duration = FACTS
        .saturating_add(STREAM_LOCK)
        .saturating_add(command(STREAM));

    /// A catalogue stream's stop: another start or stop of one to finish,
    /// then 1230.
    pub const CATALOGUE_STREAM_STOP: Duration = STREAM_LOCK.saturating_add(command(STREAM));

    /// A display-area write: 1440, and when the caller's calibration session
    /// ended meanwhile, 1440 again with the area that end put back.
    pub const DISPLAY_AREA_SET: Duration = commands(&[DISPLAY_AREA, DISPLAY_AREA]);

    /// A calibration start: 1010, 1060 and 1110, and when one fails, 1020
    /// and 1110 to put the calibration back.
    pub const CALIBRATION_START: Duration = commands(&[
        CALIBRATION_QUICK,
        CALIBRATION_QUICK,
        CALIBRATION_SLOW,
        CALIBRATION_QUICK,
        CALIBRATION_SLOW,
    ]);

    /// A calibration stop: 1020 and 1110, then either 1110 again for an
    /// engine started in place of a lost one (when the stop saves) or 1440
    /// to put the display area back (when it keeps nothing).
    pub const CALIBRATION_STOP: Duration = commands(&[
        CALIBRATION_QUICK,
        CALIBRATION_SLOW,
        longer(CALIBRATION_SLOW, DISPLAY_AREA),
    ]);

    /// Collecting a 2-D point: 1030.
    pub const CALIBRATION_COLLECT_2D: Duration = command(CALIBRATION_QUICK);

    /// Discarding a 2-D point: 1080.
    pub const CALIBRATION_DISCARD_2D: Duration = command(CALIBRATION_QUICK);

    /// Clearing the points collected: 1060.
    pub const CALIBRATION_CLEAR: Duration = command(CALIBRATION_QUICK);

    /// Computing a calibration: 1070, then 1100 to read it back.
    pub const CALIBRATION_COMPUTE: Duration = commands(&[CALIBRATION_SLOW, CALIBRATION_QUICK]);

    /// Reading the active calibration: 1100.
    pub const CALIBRATION_RETRIEVE: Duration = command(CALIBRATION_QUICK);

    /// Writing a calibration: 1110.
    pub const CALIBRATION_APPLY: Duration = command(CALIBRATION_SLOW);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_waits_its_queue_allowance_and_then_its_deadline() {
        for d in [Duration::ZERO, DISPLAY_AREA, CALIBRATION_SLOW] {
            assert_eq!(command(d), QUEUE_ALLOWANCE + d, "{d:?}");
        }
        assert_eq!(command(Duration::MAX), Duration::MAX, "saturates");
    }

    #[test]
    fn commands_run_one_after_another_wait_for_each_in_turn() {
        assert_eq!(commands(&[]), Duration::ZERO);
        assert_eq!(
            commands(&[DISPLAY_AREA, CALIBRATION_SLOW]),
            command(DISPLAY_AREA) + command(CALIBRATION_SLOW)
        );
        assert_eq!(
            commands(&[Duration::MAX, DISPLAY_AREA]),
            Duration::MAX,
            "saturates"
        );
    }

    /// A catalogue stream's start waits for the facts, then for another
    /// start or stop of one, then for its 1220, which the device has the
    /// DLL's 3 s to answer: 10 + 20 + 33 = 63 s at worst. A stop skips the
    /// facts: 20 + 33 = 53 s.
    #[test]
    fn a_catalogue_stream_change_waits_for_the_lock_and_then_its_command() {
        assert_eq!(STREAM, Duration::from_micros(0x2d_c6c0));
        assert_eq!(
            worst::CATALOGUE_STREAM_START,
            FACTS + STREAM_LOCK + command(STREAM)
        );
        assert_eq!(worst::CATALOGUE_STREAM_STOP, STREAM_LOCK + command(STREAM));
        assert_eq!(
            (worst::CATALOGUE_STREAM_START, worst::CATALOGUE_STREAM_STOP),
            (Duration::from_secs(63), Duration::from_secs(53))
        );
    }
}
