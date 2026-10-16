//! Thin client: subscribe to gaze from `tobiid` and, while the user looks at
//! the left or right edge of the screen, emit a Left/Right arrow key once per
//! interval (500 ms by default) via the kernel `uinput` device. Two quick
//! winks of one eye press Super+Left (the left eye) or Super+Right (the
//! right eye) on the same device, Super held or not. Works under both X11
//! and Wayland because the key events are injected at the kernel input
//! layer.
//!
//! Needs `/dev/uinput` and, to see Super held, the keyboards'
//! `/dev/input/event*`: the `input` group (INSTALL.md §3). Not `sudo`: as root
//! it would find no daemon of the user's and start one of its own.

use std::cmp::Ordering;
use std::io::IsTerminal;
use std::str::FromStr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use tobii_ipc::{STREAM_GAZE, ServerMsg, decode_server, encode_subscribe, read_frame, write_frame};

mod uinput {
    //! Minimal dependency-free `uinput` keyboard: a `File` on `/dev/uinput`
    //! driven with raw libc ioctls.
    use std::fs::{File, OpenOptions};
    use std::io::{self, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    use std::time::Duration;

    // ioctl request numbers (see <linux/uinput.h>, computed for x86_64).
    const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564; // _IOW('U', 100, int)
    const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565; // _IOW('U', 101, int)
    const UI_DEV_CREATE: libc::c_ulong = 0x5501; // _IO('U', 1)
    const UI_DEV_DESTROY: libc::c_ulong = 0x5502; // _IO('U', 2)

    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const SYN_REPORT: u16 = 0;
    const BUS_USB: u16 = 0x03;

    /// evdev key code of the Left arrow.
    pub const KEY_LEFT: u16 = 105;
    /// evdev key code of the Right arrow.
    pub const KEY_RIGHT: u16 = 106;
    /// evdev key code of the left Super (Meta) key.
    pub const KEY_LEFTMETA: u16 = 125;

    /// The name the virtual keyboard registers with the kernel.
    pub const NAME: &str = "tobii-gaze-keys";

    /// How long [`Keyboard::chord`] holds its modifier alone, before the tap
    /// and again after it, as a hand would (xdotool and ydotool wait 12 ms
    /// between keys).
    const CHORD_GAP: Duration = Duration::from_millis(10);

    /// `struct input_id` from `<linux/input.h>`.
    #[repr(C)]
    struct InputId {
        bustype: u16,
        vendor: u16,
        product: u16,
        version: u16,
    }

    /// `struct uinput_user_dev` from `<linux/uinput.h>`.
    #[repr(C)]
    struct UinputUserDev {
        name: [u8; 80],
        id: InputId,
        ff_effects_max: u32,
        absmax: [i32; 64],
        absmin: [i32; 64],
        absfuzz: [i32; 64],
        absflat: [i32; 64],
    }

    /// `struct input_event` on 64-bit Linux: 16-byte `timeval`, type, code, value.
    #[repr(C)]
    struct InputEvent {
        tv_sec: i64,
        tv_usec: i64,
        type_: u16,
        code: u16,
        value: i32,
    }

    /// Marker for `#[repr(C)]` structs that can be handed to the kernel as raw
    /// bytes.
    ///
    /// # Safety
    ///
    /// Implementors must be `#[repr(C)]`, contain only integer fields (and
    /// arrays/structs of them) and have no padding, so every byte of a value
    /// is initialised.
    unsafe trait PlainBytes {}

    // SAFETY: `#[repr(C)]`; 80 + 4*2 + 4 + 4*64*4 bytes of integers, all
    // naturally aligned, no padding.
    unsafe impl PlainBytes for UinputUserDev {}
    // SAFETY: `#[repr(C)]`; 8 + 8 + 2 + 2 + 4 = 24 bytes of integers, no padding.
    unsafe impl PlainBytes for InputEvent {}

    /// View a kernel ABI struct as the bytes the driver expects on `write(2)`.
    fn as_bytes<T: PlainBytes>(v: &T) -> &[u8] {
        // SAFETY: `PlainBytes` guarantees `T` has no padding or non-integer
        // fields, so all `size_of::<T>()` bytes behind `v` are initialised; the
        // borrow keeps them alive and unaliased-mutably for the returned lifetime.
        unsafe { std::slice::from_raw_parts(std::ptr::from_ref(v).cast::<u8>(), size_of::<T>()) }
    }

    /// `ioctl(fd, req, arg)` for the `_IOW(int)`-style uinput requests.
    fn ioctl_int(file: &File, req: libc::c_ulong, arg: libc::c_int) -> io::Result<()> {
        // SAFETY: `file` keeps the descriptor open for the duration of the call
        // and the request takes a plain `int` by value — no pointer is passed.
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), req, arg) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// `ioctl(fd, req)` for the argument-less `_IO`-style uinput requests.
    fn ioctl_none(file: &File, req: libc::c_ulong) -> io::Result<()> {
        // SAFETY: `file` keeps the descriptor open for the duration of the call
        // and the request takes no argument.
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), req) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// A virtual keyboard registered with the kernel; unregistered on drop.
    pub struct Keyboard {
        file: File,
    }

    impl Keyboard {
        /// Create a virtual keyboard exposing the given keys.
        pub fn new(keys: &[u16]) -> io::Result<Self> {
            let file = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open("/dev/uinput")?;
            // From here on `Drop` issues UI_DEV_DESTROY on any early return.
            let kb = Keyboard { file };

            ioctl_int(&kb.file, UI_SET_EVBIT, libc::c_int::from(EV_KEY))?;
            ioctl_int(&kb.file, UI_SET_EVBIT, libc::c_int::from(EV_SYN))?;
            for &k in keys {
                ioctl_int(&kb.file, UI_SET_KEYBIT, libc::c_int::from(k))?;
            }

            let mut dev = UinputUserDev {
                name: [0; 80],
                id: InputId {
                    bustype: BUS_USB,
                    vendor: 0x1234,
                    product: 0x5678,
                    version: 1,
                },
                ff_effects_max: 0,
                absmax: [0; 64],
                absmin: [0; 64],
                absfuzz: [0; 64],
                absflat: [0; 64],
            };
            let name = NAME.as_bytes();
            dev.name[..name.len()].copy_from_slice(name);
            (&kb.file).write_all(as_bytes(&dev))?;
            ioctl_none(&kb.file, UI_DEV_CREATE)?;
            // Give udev/compositor a moment to register the new device.
            std::thread::sleep(Duration::from_millis(200));
            Ok(kb)
        }

        fn emit(&self, type_: u16, code: u16, value: i32) -> io::Result<()> {
            let ev = InputEvent {
                tv_sec: 0,
                tv_usec: 0,
                type_,
                code,
                value,
            };
            (&self.file).write_all(as_bytes(&ev))
        }

        /// Press (`down`) or release one key, as a report of its own.
        fn key(&self, key: u16, down: bool) -> io::Result<()> {
            self.emit(EV_KEY, key, i32::from(down))?;
            self.emit(EV_SYN, SYN_REPORT, 0)
        }

        /// Press and release one key.
        pub fn tap(&self, key: u16) -> io::Result<()> {
            self.key(key, true)?;
            self.key(key, false)
        }

        /// Press and release `key` while `modifier` is held: the modifier
        /// down, [`CHORD_GAP`], the tap, [`CHORD_GAP`], the modifier up. The
        /// modifier is released even when pressing it or the tap failed, so
        /// that it is not left held for whatever the user types next; the
        /// error returned is the first one.
        pub fn chord(&self, modifier: u16, key: u16) -> io::Result<()> {
            let tapped = self.key(modifier, true).and_then(|()| {
                std::thread::sleep(CHORD_GAP);
                self.tap(key)
            });
            std::thread::sleep(CHORD_GAP);
            let released = self.key(modifier, false);
            tapped.and(released)
        }
    }

    impl Drop for Keyboard {
        fn drop(&mut self) {
            // The descriptor itself is closed when `file` drops right after this.
            if let Err(e) = ioctl_none(&self.file, UI_DEV_DESTROY) {
                tracing::warn!(error = %e, "UI_DEV_DESTROY failed");
            }
        }
    }
}

mod superkey {
    //! Track whether a Super (Meta) key is physically held, by reading evdev
    //! `/dev/input/event*` in background threads. Works under X11 and Wayland.
    use std::fs;
    use std::io::Read;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    const EV_KEY: u16 = 0x01;
    const KEY_LEFTMETA: u16 = 125;
    const KEY_RIGHTMETA: u16 = 126;

    /// Live state of the two Meta keys, fed by one reader thread per device.
    pub struct Watcher {
        left: Arc<AtomicBool>,
        right: Arc<AtomicBool>,
    }

    impl Watcher {
        /// Open every readable input device and watch it for Meta key events.
        pub fn spawn() -> std::io::Result<Self> {
            let left = Arc::new(AtomicBool::new(false));
            let right = Arc::new(AtomicBool::new(false));
            let mut opened = 0;
            for entry in fs::read_dir("/dev/input")? {
                let entry = entry?;
                let node = entry.file_name();
                let node = node.to_string_lossy();
                if !node.starts_with("event") {
                    continue;
                }
                // The virtual keyboard of this program, or of a second
                // instance: a Super pressed through it is not one the user
                // holds, and its release must not clear the user's.
                let name = fs::read_to_string(format!("/sys/class/input/{node}/device/name"))
                    .unwrap_or_default();
                if name.trim_end() == super::uinput::NAME {
                    continue;
                }
                // Some nodes aren't readable by us (or are busy) — skip them.
                let Ok(file) = fs::File::open(entry.path()) else {
                    continue;
                };
                opened += 1;
                let (l, r) = (Arc::clone(&left), Arc::clone(&right));
                thread::spawn(move || reader_loop(file, &l, &r));
            }
            if opened == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "no readable /dev/input/event* devices (join the `input` group?)",
                ));
            }
            Ok(Watcher { left, right })
        }

        /// True while either Super/Meta key is currently down.
        pub fn is_down(&self) -> bool {
            // Relaxed: the flags are pure signals; no other data is published
            // through them.
            self.left.load(Ordering::Relaxed) || self.right.load(Ordering::Relaxed)
        }
    }

    fn reader_loop(mut file: fs::File, left: &AtomicBool, right: &AtomicBool) {
        // input_event on 64-bit: 16B timeval + u16 type + u16 code + i32 value.
        let mut buf = [0u8; 24];
        while file.read_exact(&mut buf).is_ok() {
            if u16::from_ne_bytes([buf[16], buf[17]]) != EV_KEY {
                continue;
            }
            let code = u16::from_ne_bytes([buf[18], buf[19]]);
            let value = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
            let down = value != 0; // 1=press, 2=autorepeat, 0=release
            match code {
                KEY_LEFTMETA => left.store(down, Ordering::Relaxed),
                KEY_RIGHTMETA => right.store(down, Ordering::Relaxed),
                _ => {}
            }
        }
    }
}

mod wink {
    //! Double winks: one eye closed twice in quick succession, the other open.
    //!
    //! A pure state machine fed once per GAZE frame, with no clock of its own:
    //! every time in it is a difference of the frames' `ts_us`, which come
    //! 30.2 ms apart (33.1 Hz), also while the eyes are lost.
    //!
    //! An episode is a maximal run of frames with an eye lost, judged on the
    //! frame that has both eyes back. It is a wink when one eye alone was
    //! lost (the open eye may squint for up to `MAX_BOTH` frames) for at
    //! least `MIN_SINGLE_US`, it lasted `min_closed_us..=max_closed_us`, and
    //! the gaze was on screen right before it. A wink after at least `QUIET_US` of
    //! open eyes waits for a wink of the same eye, which completes the double
    //! when the eyes were open at most `max_gap_us` between the two, however
    //! briefly: a quick double opens the eye for a frame or two. Any other
    //! episode, and a gap in the stream, starts over. Each bound falls
    //! between two whole numbers of frames.

    /// Least time both eyes are open before the first wink of a double: 5
    /// frames (151 ms) pass, 4 (121 ms) do not. A retry right after a double
    /// that did not count comes about that soon.
    const QUIET_US: i64 = 135_000;

    /// Most frames with both eyes lost that a wink may hold: the open eye
    /// squints at times.
    const MAX_BOTH: u32 = 2;

    /// Least time the winking eye is lost alone within a wink: 3 frames
    /// (91 ms) pass, 2 (60 ms) do not. An episode with both eyes lost for
    /// nearly all of it is a blink with a ragged edge.
    const MIN_SINGLE_US: i64 = 80_000;

    /// Longest step between two frames that follow each other. The tracker
    /// sends a frame every 30.2 ms whatever the eyes do, so a longer step is
    /// a pause, a re-open or a calibration, and time that does not go forward
    /// is a new stream: either way, what was under way is unknown.
    const MAX_STEP_US: i64 = 100_000;

    /// One of the user's eyes, as the gaze stream names them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Eye {
        /// The user's left eye.
        Left,
        /// The user's right eye.
        Right,
    }

    /// The bounds set on the command line, microseconds of `ts_us`.
    #[derive(Debug, Clone, Copy)]
    pub struct Config {
        /// Least time a wink lasts, from its first frame with an eye lost to
        /// the reopening, the open eye's squint included; by default 135 ms,
        /// which 5 frames (151 ms) pass and 4 (121 ms) do not. The tracker
        /// drops the eye on the side the gaze is turned to for 3 or 4 frames
        /// at a time, often in runs; a wink is longer.
        pub min_closed_us: i64,
        /// Longest a wink lasts, from its first frame with an eye lost to the
        /// reopening; by default 590 ms, which 19 frames (574 ms) pass and 20
        /// (604 ms) do not.
        pub max_closed_us: i64,
        /// Longest time both eyes are open between the two winks of a
        /// double; by default 800 ms, which 26 frames (785 ms) pass and 27
        /// (816 ms) do not.
        pub max_gap_us: i64,
    }

    impl Default for Config {
        fn default() -> Self {
            Config {
                min_closed_us: 135_000,
                max_closed_us: 590_000,
                max_gap_us: 800_000,
            }
        }
    }

    /// What one frame shows of the eyes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Class {
        /// Both eyes tracked.
        Open,
        /// Only this eye lost.
        Single(Eye),
        /// Both eyes lost.
        Both,
    }

    /// A run of frames with an eye lost, as far as it has come.
    #[derive(Debug)]
    struct Episode {
        /// `ts_us` of its first frame.
        start_us: i64,
        /// How long both eyes were open before it, counted from the first
        /// frame of that stretch seen since the stream's start or the last
        /// gap (a stretch cut by either counts only its seen part); `None` if
        /// no frame of it was seen (the episode begins on the stream's first
        /// frame or on the first frame after a gap) or its last frame had the
        /// gaze off screen. A `None` keeps the episode out of a double: it
        /// neither fires nor becomes pending.
        open_us: Option<i64>,
        /// The eye of its first one-eye frame.
        eye: Option<Eye>,
        /// Whether the other eye was lost alone as well.
        mixed: bool,
        /// How many of its frames had both eyes lost.
        both: u32,
        /// How long `eye` was lost alone: each such frame counts the step to
        /// the frame after it.
        single_us: i64,
    }

    /// Finds double winks in the GAZE frames, one frame at a time.
    #[derive(Debug)]
    pub struct Detector {
        /// The bounds.
        cfg: Config,
        /// `ts_us` and class of the frame before.
        last: Option<(i64, Class)>,
        /// `ts_us` of the first frame of the current stretch with both eyes
        /// open; `None` during an episode, and after the stream's start or a
        /// gap until the next frame with both eyes open.
        open_since: Option<i64>,
        /// Whether the gaze was on screen in the last frame with both eyes
        /// open.
        on_screen: bool,
        /// The episode under way.
        episode: Option<Episode>,
        /// The eye of a first wink that the current open stretch follows,
        /// waiting for the second.
        pending: Option<Eye>,
    }

    impl Detector {
        /// A detector that has seen no frame yet.
        #[must_use]
        pub fn new(cfg: Config) -> Self {
            Detector {
                cfg,
                last: None,
                open_since: None,
                on_screen: false,
                episode: None,
                pending: None,
            }
        }

        /// Take the next GAZE frame: its `ts_us`, which eyes are tracked and
        /// whether its gaze point is on screen (read only with both eyes
        /// tracked). `Some(eye)` when the frame is the reopening that
        /// completes a double wink of `eye`.
        #[must_use]
        pub fn push(
            &mut self,
            ts_us: i64,
            left_open: bool,
            right_open: bool,
            on_screen: bool,
        ) -> Option<Eye> {
            let class = match (left_open, right_open) {
                (true, true) => Class::Open,
                (false, true) => Class::Single(Eye::Left),
                (true, false) => Class::Single(Eye::Right),
                (false, false) => Class::Both,
            };
            if let Some((last_us, last_class)) = self.last.replace((ts_us, class)) {
                let step_us = ts_us.saturating_sub(last_us);
                if !(1..=MAX_STEP_US).contains(&step_us) {
                    // Frames are missing, or time went back: start over.
                    self.episode = None;
                    self.pending = None;
                    self.open_since = None;
                } else if let Some(episode) = &mut self.episode
                    && episode
                        .eye
                        .is_some_and(|eye| last_class == Class::Single(eye))
                {
                    // The step belongs to the frame before it.
                    episode.single_us = episode.single_us.saturating_add(step_us);
                }
            }
            match class {
                Class::Open => {
                    let fired = self
                        .episode
                        .take()
                        .and_then(|episode| self.end(&episode, ts_us));
                    self.open_since = self.open_since.or(Some(ts_us));
                    self.on_screen = on_screen;
                    fired
                }
                Class::Single(eye) => {
                    let episode = self.episode_at(ts_us);
                    if *episode.eye.get_or_insert(eye) != eye {
                        episode.mixed = true;
                    }
                    None
                }
                Class::Both => {
                    let episode = self.episode_at(ts_us);
                    episode.both = episode.both.saturating_add(1);
                    None
                }
            }
        }

        /// The episode under way, begun at `ts_us` if there is none.
        fn episode_at(&mut self, ts_us: i64) -> &mut Episode {
            self.episode.get_or_insert_with(|| Episode {
                start_us: ts_us,
                open_us: self
                    .open_since
                    .take()
                    .filter(|_| self.on_screen)
                    .map(|since| ts_us.saturating_sub(since)),
                eye: None,
                mixed: false,
                both: 0,
                single_us: 0,
            })
        }

        /// Judge the episode that the frame at `ts_us`, with both eyes open,
        /// ended: `Some(eye)` if it was the second wink of a double.
        fn end(&mut self, episode: &Episode, ts_us: i64) -> Option<Eye> {
            let closed_us = ts_us.saturating_sub(episode.start_us);
            let wink = episode.eye.filter(|_| {
                !episode.mixed
                    && episode.both <= MAX_BOTH
                    && episode.single_us >= MIN_SINGLE_US
                    && (self.cfg.min_closed_us..=self.cfg.max_closed_us).contains(&closed_us)
            });
            let (fired, pending) = match (wink, episode.open_us) {
                (Some(eye), Some(open_us))
                    if self.pending == Some(eye) && open_us <= self.cfg.max_gap_us =>
                {
                    (Some(eye), None)
                }
                (Some(eye), Some(open_us)) if open_us >= QUIET_US => (None, Some(eye)),
                _ => (None, None),
            };
            tracing::debug!(
                eye = ?episode.eye,
                mixed = episode.mixed,
                both = episode.both,
                single_ms = episode.single_us / 1000,
                closed_ms = closed_us / 1000,
                open_before_ms = ?episode.open_us.map(|us| us / 1000),
                wink = wink.is_some(),
                ?fired,
                ?pending,
                "eye-lost episode"
            );
            self.pending = pending;
            fired
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The tracker's frame period: 33.1 frames a second.
        const STEP_US: i64 = 30_208;

        /// No fire.
        const NONE: [(usize, Eye); 0] = [];

        /// The user's 150 s recording (2026-10-12) frame by frame: settle,
        /// 5 left and 5 right double winks, natural double blinks, single
        /// winks of each eye (20 to 35 frames), then idle with looks off
        /// screen.
        const SESSION: &str = "\
            .338 l5 .14 l7 .40 l4 .13 l7 .41 l9 .10 l6 .37 l8 .11 l3 b1 l5 .39 l7 .12 l6 \
            .156 r10 .11 r9 .36 r8 .11 r8 .29 r12 .13 r9 .40 r9 .9 r12 .30 r8 .15 r8 \
            .42 b5 .15 b5 l1 .58 b5 .17 b5 .49 b7 .16 b5 .64 b5 .16 b4 .51 b8 .14 b4 \
            .91 l5 b2 l1 b2 l3 b1 l5 .49 l1 b1 l19 .55 l20 .65 r21 .61 r22 .66 r35 \
            .5 b2 l1 .281 b3 .48 b4 .99 b2 l1 .151 b3 .138 b1 .234 b3 .170 b3 l1 \
            .197 r1 .246 b5 .160 l2 .1 b2 l1 b2 l1 r1 b107 .80 b3 r1 .260 b4 .177 \
            b4 r1 .243 b3 r1 .270 b3 l1 .69";

        /// `pattern` one character per frame: space-separated runs as the
        /// recordings print them (`l5` is 5 frames of `l`) or literal frames
        /// (`lllblllll`).
        fn expand(pattern: &str) -> String {
            pattern
                .split_whitespace()
                .map(|token| {
                    let mut chars = token.chars();
                    match (chars.next(), chars.as_str().parse::<usize>()) {
                        (Some(frame), Ok(n)) => frame.to_string().repeat(n),
                        _ => token.to_owned(),
                    }
                })
                .collect()
        }

        /// Feed `pattern` (see `expand`) to a detector with the default
        /// bounds, the first frame at `start_us` and one every `STEP_US`, and
        /// return what fired as (frame index, eye). A frame is `.` (both eyes
        /// tracked, gaze on screen), `o` (both tracked, gaze off screen), `l`
        /// or `r` (only that eye lost) or `b` (both lost). `|`, `=` and `<`
        /// are no frames: the frame after them comes 1 s late, at the time of
        /// the frame before, or 2 s early.
        fn fires_from(start_us: i64, pattern: &str) -> Vec<(usize, Eye)> {
            let mut det = Detector::new(Config::default());
            let mut next_us = start_us;
            let mut index = 0;
            let mut fired = Vec::new();
            for frame in expand(pattern).chars() {
                let (left, right, on_screen) = match frame {
                    '|' => {
                        next_us += 1_000_000;
                        continue;
                    }
                    '=' => {
                        next_us -= STEP_US;
                        continue;
                    }
                    '<' => {
                        next_us -= 2_000_000;
                        continue;
                    }
                    '.' => (true, true, true),
                    'o' => (true, true, false),
                    'l' => (false, true, true),
                    'r' => (true, false, true),
                    'b' => (false, false, false),
                    other => panic!("no such frame: {other:?}"),
                };
                if let Some(eye) = det.push(next_us, left, right, on_screen) {
                    fired.push((index, eye));
                }
                next_us += STEP_US;
                index += 1;
            }
            fired
        }

        /// `fires_from` at an ordinary time.
        fn fires(pattern: &str) -> Vec<(usize, Eye)> {
            fires_from(1_000_000_000, pattern)
        }

        #[test]
        fn a_double_wink_fires_on_the_second_reopening() {
            assert_eq!(fires(".15 l5 .14 l7 .5"), [(41, Eye::Left)]);
            assert_eq!(fires(".15 r10 .11 r9 .5"), [(45, Eye::Right)]);
        }

        #[test]
        fn the_recorded_doubles_fire() {
            let left =
                ".172 l5 .14 l7 .40 l4 .13 l7 .41 l9 .10 l6 .37 l8 .11 l3 b1 l5 .39 l7 .12 l6 .20";
            // The second double opens with a 4-frame (121 ms) wink.
            assert_eq!(
                fires(left),
                [198, 328, 393, 457].map(|index| (index, Eye::Left))
            );
            let right =
                ".40 r10 .11 r9 .36 r8 .11 r8 .29 r12 .13 r9 .40 r9 .9 r12 .30 r8 .15 r8 .20";
            assert_eq!(
                fires(right),
                [70, 133, 196, 266, 327].map(|index| (index, Eye::Right))
            );
        }

        #[test]
        fn the_recorded_session_fires_nine_of_its_doubles_and_nothing_else() {
            // The second left double opens with a 4-frame (121 ms) wink.
            let left = [364, 494, 559, 623].map(|index| (index, Eye::Left));
            let right = [809, 872, 935, 1005, 1066].map(|index| (index, Eye::Right));
            assert_eq!(expand(SESSION).len(), 4983);
            assert_eq!(fires(SESSION), [&left[..], &right[..]].concat());
        }

        #[test]
        fn the_open_eye_may_squint_twice() {
            assert_eq!(fires(".37 l8 .11 lllblllll .5"), [(65, Eye::Left)]);
            assert_eq!(fires(".15 blllllb .12 blllllb ."), [(41, Eye::Left)]);
            assert_eq!(fires(".15 bblll .12 lllbb ."), [(37, Eye::Left)]);
            assert_eq!(fires(".15 bbblll .12 bbblll .20"), NONE);
        }

        #[test]
        fn a_single_wink_does_not_fire() {
            assert_eq!(fires(".15 l7 .30"), NONE);
        }

        #[test]
        fn natural_blinks_do_not_fire() {
            for blinks in [
                ".40 b5 .15 b5 .40",
                ".40 bbbbbl .15 bbbbb .40",
                ".40 bbl .15 bbl .40",
                ".40 lbbb .15 lbbb .40",
                ".42 b5 .15 b5 l1 .58 b5 .17 b5 .49 b7 .16 b5 .28",
            ] {
                assert_eq!(fires(blinks), NONE, "{blinks}");
            }
        }

        #[test]
        fn a_wink_is_five_frames_at_least() {
            assert_eq!(fires(".15 l5 .8 l5 ."), [(33, Eye::Left)]);
            // The open eye's squint counts towards the length.
            assert_eq!(fires(".15 lbbll .8 lbbll ."), [(33, Eye::Left)]);
            for short in [
                ".15 l4 .8 l4 .",
                ".15 l3 .8 l3 .",
                ".15 lbbl .8 lbbl .",
                ".15 l2 .8 l2 .",
                ".15 l . l .15",
                ".15 ll .. ll .15",
                ".15 l .8 l .15",
            ] {
                assert_eq!(fires(short), NONE, "{short}");
            }
        }

        #[test]
        fn the_trackers_short_drops_of_one_eye_do_not_fire() {
            // As the tracker lost the left eye on 2026-10-15 and 16, the gaze
            // turned to that side or the eye flickering: 3 or 4 frames at a
            // time, in pairs a quick double has.
            for drops in [
                ".40 lll .3 lll .20",
                ".40 llll .6 lll .20",
                ".40 l16 . llll .20",
                ".40 lll .2 llll .20",
            ] {
                assert_eq!(fires(drops), NONE, "{drops}");
            }
            // The doubles the user undid them with, from the same days.
            assert_eq!(fires(".40 rrrrrr .2 rrrrrr ."), [(54, Eye::Right)]);
            assert_eq!(fires(".40 lblbll .2 llllll ."), [(54, Eye::Left)]);
        }

        #[test]
        fn a_wink_is_nineteen_frames_at_most() {
            assert_eq!(fires(".40 l19 .10 l5 ."), [(74, Eye::Left)]);
            assert_eq!(fires(".40 l20 .10 l5 .20"), NONE);
            assert_eq!(fires(".40 l5 .10 l20 .20"), NONE);
            // The recorded single winks.
            assert_eq!(
                fires(".49 l1 b1 l19 .55 l20 .65 r21 .61 r22 .66 r35 .5 b2 l1 .50"),
                NONE
            );
        }

        #[test]
        fn the_winks_pause_up_to_twenty_six_open_frames() {
            assert_eq!(fires(".15 l5 . l5 ."), [(26, Eye::Left)]);
            assert_eq!(fires(".15 l5 .26 l5 ."), [(51, Eye::Left)]);
            assert_eq!(fires(".15 l5 .27 l5 .5"), NONE);
            // The late second wink is a first one itself.
            assert_eq!(fires(".15 l5 .27 l5 .10 l5 ."), [(67, Eye::Left)]);
        }

        #[test]
        fn quick_doubles_fire() {
            // As the live check of 2026-10-12 saw them: the eye open for one
            // to three frames between the winks, once with a squint.
            assert_eq!(fires(".20 rrrrrrb . r8 ."), [(36, Eye::Right)]);
            assert_eq!(fires(".20 r5 .. r7 ."), [(34, Eye::Right)]);
            assert_eq!(fires(".20 l9 ... l9 ."), [(41, Eye::Left)]);
        }

        #[test]
        fn the_first_wink_follows_five_open_frames() {
            assert_eq!(fires(".40 b5 .4 l5 .12 l5 .20"), NONE);
            assert_eq!(fires(".40 b5 .5 l5 .12 l5 ."), [(72, Eye::Left)]);
            assert_eq!(fires(".4 l5 .12 l5 .20"), NONE);
            assert_eq!(fires(".5 l5 .12 l5 ."), [(27, Eye::Left)]);
        }

        #[test]
        fn both_winks_are_of_one_eye() {
            assert_eq!(fires(".15 l5 .12 r5 .20"), NONE);
            assert_eq!(fires(".15 l5 .12 l5 .12 r5 .20"), [(37, Eye::Left)]);
            assert_eq!(fires(".15 l5 .12 r5 .12 r5 ."), [(54, Eye::Right)]);
            assert_eq!(fires(".15 lrl .12 l5 .20"), NONE);
            assert_eq!(fires(".15 l5 .12 lllrl .20"), NONE);
        }

        #[test]
        fn the_gaze_is_on_screen_right_before_each_wink() {
            assert_eq!(fires(".15 o l5 .12 l5 .20"), NONE);
            assert_eq!(fires(".15 l5 o12 l5 .20"), NONE);
            assert_eq!(fires(".15 l5 o6 .6 l5 ."), [(37, Eye::Left)]);
            assert_eq!(fires("o15 . l5 .12 l5 ."), [(38, Eye::Left)]);
        }

        #[test]
        fn anything_between_the_winks_breaks_the_double() {
            assert_eq!(fires(".15 l5 .12 b5 .12 l5 .20"), NONE);
            assert_eq!(fires(".15 l5 .5 l .5 l5 .20"), NONE);
            assert_eq!(fires(".15 l5 .5 b100 .5 l5 .20"), NONE);
            assert_eq!(fires(".15 l5 .5 b100 .15 l5 .12 l5 ."), [(162, Eye::Left)]);
        }

        #[test]
        fn a_gap_in_the_stream_starts_over() {
            for gap in [
                ".15 l5 .6 | .6 l5 .20",
                ".15 l5 .6 | .12 l5 .20",
                ".15 l3 | l2 .12 l5 .20",
                ".15 l5 .12 l3 | l3 .20",
                ".15 l5 .12 l5 | .20",
                ".15 l5 | .4 l5 .12 l5 .20",
                ".15 l5 .6 = .6 l5 .20",
                ".15 l5 .12 l3 < l3 .20",
            ] {
                assert_eq!(fires(gap), NONE, "{gap}");
            }
            assert_eq!(fires(".15 l5 | .15 l5 .12 l5 ."), [(57, Eye::Left)]);
            assert_eq!(fires(".15 l5 .6 = .15 l5 .12 l5 ."), [(63, Eye::Left)]);
        }

        #[test]
        fn a_stream_may_start_with_an_eye_lost() {
            assert_eq!(fires("l4 .15 l5 .12 l5 ."), [(41, Eye::Left)]);
            assert_eq!(fires("l4 .12 l5 .20"), NONE);
        }

        #[test]
        fn more_winks_fire_once_per_pair() {
            assert_eq!(fires(".15 l5 .12 l5 .12 l5 .20"), [(37, Eye::Left)]);
            assert_eq!(
                fires(".15 l5 .12 l5 .12 l5 .12 l5 ."),
                [(37, Eye::Left), (71, Eye::Left)]
            );
            // A third wink 4 open frames after a fire cannot begin a double.
            assert_eq!(fires(".15 l5 .4 l5 .4 l5 .4 l5 ."), [(29, Eye::Left)]);
        }

        #[test]
        fn extreme_timestamps_do_not_overflow() {
            assert_eq!(fires_from(i64::MIN, ".15 l5 .14 l7 ."), [(41, Eye::Left)]);
            assert_eq!(
                fires_from(i64::MAX - 2_000_000, ".15 l5 .14 l7 ."),
                [(41, Eye::Left)]
            );
            let mut det = Detector::new(Config::default());
            for ts_us in [i64::MIN, i64::MAX, i64::MIN, 0, i64::MAX] {
                assert_eq!(det.push(ts_us, false, true, true), None);
                assert_eq!(det.push(ts_us, true, true, true), None);
            }
        }
    }
}

fn main() {
    tobii_log::init();
    if let Err(e) = run() {
        eprintln!("tobii-gaze-keys: {e:#}");
        std::process::exit(1);
    }
}

/// Take and parse the value following `flag`.
fn next_value<T>(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let v = args
        .next()
        .with_context(|| format!("{flag} needs a value"))?;
    v.parse()
        .with_context(|| format!("{flag}: invalid value {v:?}"))
}

/// Milliseconds from the command line as microseconds of `ts_us`.
fn ms_to_us(ms: u32) -> i64 {
    i64::from(ms) * 1000
}

/// Valid gaze points in a row with no pupil for either eye, about a second's
/// worth, after which the binder takes tobiid for one that sends no pupils:
/// one older than this binary. A current tobiid sends the pupil of every eye
/// it tracks, and a valid gaze point needs a tracked eye.
const NO_PUPILS_FRAMES: u32 = 33;

/// How far past the screen's edges, in screen widths and heights, a gaze
/// point still counts as on the screen before a wink: the edges' noise, not a
/// look past them, which the tracker puts 0.1 to 0.2 beyond.
const SCREEN_SLACK: f32 = 0.05;

fn run() -> Result<()> {
    // Manual thresholds (absolute gaze-X). Left unset => adaptive auto-calibration,
    // which tracks the actually-observed gaze-X range and triggers near its edges.
    // This handles trackers whose gaze-X isn't centered/symmetric (a fixed 0.15
    // left edge may simply be unreachable while 0.85 right is).
    let mut left_edge: Option<f32> = None;
    let mut right_edge: Option<f32> = None;
    let mut margin = 0.25f32; // auto: zone width as a fraction of the observed span
    let mut interval = Duration::from_millis(500);
    let mut require_super = true; // only tap while a Super/Meta key is held
    let mut winks_on = true; // a double wink presses Super+Left/Right
    let mut wink_cfg = wink::Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--left" => left_edge = Some(next_value(&mut args, "--left")?),
            "--right" => right_edge = Some(next_value(&mut args, "--right")?),
            "--margin" => margin = next_value(&mut args, "--margin")?,
            "--no-super" => require_super = false,
            "--interval-ms" => {
                interval = Duration::from_millis(next_value(&mut args, "--interval-ms")?);
            }
            "--no-winks" => winks_on = false,
            "--wink-min-ms" => {
                wink_cfg.min_closed_us = ms_to_us(next_value(&mut args, "--wink-min-ms")?);
            }
            "--wink-max-ms" => {
                wink_cfg.max_closed_us = ms_to_us(next_value(&mut args, "--wink-max-ms")?);
            }
            "--wink-gap-ms" => {
                wink_cfg.max_gap_us = ms_to_us(next_value(&mut args, "--wink-gap-ms")?);
            }
            "-h" | "--help" => {
                println!(
                    "usage: tobii-gaze-keys [--margin 0.25] [--interval-ms 500] [--left X --right X] [--no-super]\n       \
                     [--no-winks] [--wink-min-ms 135] [--wink-max-ms 590] [--wink-gap-ms 800]\n\
                     \n\
                     Watches gaze X. While gaze is at the left/right edge of the screen\n\
                     AND a Super/Meta key is held, taps the Left/Right arrow key once per\n\
                     interval. Pass --no-super to drop the Super requirement. Needs\n\
                     /dev/uinput + /dev/input read access: the `input` group (INSTALL.md §3).\n\
                     \n\
                     Watches each eye too. Two quick winks of the left eye press Super+Left,\n\
                     two of the right eye Super+Right, Super held or not. A wink closes one\n\
                     eye alone for --wink-min-ms to --wink-max-ms; between the two winks both\n\
                     eyes are open for at most --wink-gap-ms, one frame being enough. Blinks\n\
                     and single winks do nothing. Pass --no-winks to turn this off.\n\
                     \n\
                     Default = AUTO calibration: it learns the gaze-X range you actually\n\
                     reach (look fully left and fully right once) and fires when gaze is\n\
                     within --margin of either observed extreme. Robust to a gaze stream\n\
                     that isn't centered/symmetric.\n\
                     \n\
                     --left/--right switch to FIXED absolute thresholds (0=left .. 1=right)."
                );
                return Ok(());
            }
            other => bail!("unknown arg: {other}"),
        }
    }
    // Fixed thresholds as soon as either side is given (the other side gets a
    // sane default); otherwise auto calibration.
    let edges = match (left_edge, right_edge) {
        (None, None) => None,
        (l, r) => {
            let (l, r) = (l.unwrap_or(0.15), r.unwrap_or(0.85));
            // `partial_cmp` so that a NaN on either side is rejected as well.
            ensure!(
                l.partial_cmp(&r) == Some(Ordering::Less),
                "--left must be less than --right"
            );
            Some((l, r))
        }
    };
    ensure!(
        margin > 0.0 && margin < 0.5,
        "--margin must be between 0 and 0.5"
    );
    ensure!(
        wink_cfg.min_closed_us < wink_cfg.max_closed_us,
        "--wink-min-ms must be less than --wink-max-ms"
    );
    ensure!(wink_cfg.max_gap_us > 0, "--wink-gap-ms must be more than 0");

    let kb = uinput::Keyboard::new(&[uinput::KEY_LEFT, uinput::KEY_RIGHT, uinput::KEY_LEFTMETA])
        .context("opening /dev/uinput (needs the `input` group: INSTALL.md §3)")?;

    // Watch the physical Super/Meta key (unless gating is disabled).
    let supers = if require_super {
        Some(superkey::Watcher::spawn().context(
            "watching modifier keys (needs read access to /dev/input/event*: the \
                     `input` group, INSTALL.md §3)",
        )?)
    } else {
        None
    };

    let mut stream = tobii_ipc::connect_or_spawn().context("connecting to tobiid")?;
    write_frame(&mut stream, &encode_subscribe(STREAM_GAZE))
        .context("subscribing to the gaze stream")?;

    let gate = if require_super {
        " while Super held"
    } else {
        ""
    };
    if let Some((l, r)) = edges {
        println!(
            "tobii-gaze-keys: fixed edges <{l:.2} / >{r:.2}, 1 tap / {}ms{gate}",
            interval.as_millis()
        );
    } else {
        println!(
            "tobii-gaze-keys: AUTO calibration (margin {margin:.2}), 1 tap / {}ms{gate}\n\
             look fully LEFT and fully RIGHT once to learn your gaze range...",
            interval.as_millis()
        );
    }
    let mut winks = winks_on.then(|| wink::Detector::new(wink_cfg));
    if winks.is_some() {
        println!("tobii-gaze-keys: a double wink of the left/right eye presses Super+Left/Right");
    }

    // The status line rewrites itself with \r for every gaze sample, which
    // only a terminal shows as one line: run as a service, stderr is the
    // journal, which would gather it into lines that grow without end, 33
    // samples a second. With the winks' debug lines on, it would garble
    // those as well.
    let live = std::io::stderr().is_terminal()
        && !(winks.is_some()
            && tracing::enabled!(target: "tobii_gaze_keys::wink", tracing::Level::DEBUG));
    let status = |line: std::fmt::Arguments<'_>| {
        if live {
            eprint!("\r{line}        ");
        }
    };

    // Rate-limit each direction independently; fire immediately on entering a zone.
    let mut last_left: Option<Instant> = None;
    let mut last_right: Option<Instant> = None;

    // Adaptive envelope of observed gaze-X (auto mode). We need at least this much
    // span before triggering, so a stationary gaze can't fire from noise alone.
    const MIN_SPAN: f32 = 0.20;
    let mut xmin = f32::INFINITY;
    let mut xmax = f32::NEG_INFINITY;

    // Valid gaze points in a row with no pupil for either eye.
    let mut no_pupils = 0u32;

    loop {
        let Some(body) = read_frame(&mut stream).context("reading from tobiid")? else {
            bail!("daemon closed the connection");
        };
        match decode_server(&body) {
            Some(ServerMsg::Subscribed { ok: false }) => {
                bail!(
                    "tobiid refused the gaze subscription; only a tobiid older than this \
                     tobii-gaze-keys refuses one (restart it after installing)"
                )
            }
            Some(ServerMsg::Subscribed { ok: true }) => {}
            Some(ServerMsg::Gaze {
                ts_us,
                valid,
                xy,
                pupil_mm,
            }) => {
                let now = Instant::now();

                // Winks first: they need every frame, the invalid ones of two
                // closed eyes included. An eye without a pupil is closed, or
                // lost.
                let [left_open, right_open] = pupil_mm.map(|mm| !mm.is_nan());
                no_pupils = if valid && !left_open && !right_open {
                    no_pupils.saturating_add(1)
                } else {
                    0
                };
                if no_pupils == NO_PUPILS_FRAMES && winks.take().is_some() {
                    if live {
                        eprintln!(); // end the status line
                    }
                    tracing::warn!(
                        "tobiid sends no pupils, as one older than this tobii-gaze-keys does \
                         (restart it after installing): double winks are off"
                    );
                }
                let on_screen = valid
                    && xy
                        .iter()
                        .all(|v| (-SCREEN_SLACK..=1.0 + SCREEN_SLACK).contains(v));
                if let Some(detector) = &mut winks
                    && let Some(eye) = detector.push(ts_us, left_open, right_open, on_screen)
                {
                    let (key, side) = match eye {
                        wink::Eye::Left => (uinput::KEY_LEFT, "Left"),
                        wink::Eye::Right => (uinput::KEY_RIGHT, "Right"),
                    };
                    kb.chord(uinput::KEY_LEFTMETA, key)
                        .with_context(|| format!("pressing Super+{side}"))?;
                    if live {
                        eprintln!(); // end the status line: the event gets its own
                    }
                    tracing::info!("double wink: Super+{side}");
                }

                if !valid {
                    // Lost gaze: reset so re-entry taps right away.
                    last_left = None;
                    last_right = None;
                    continue;
                }
                let x = xy[0];

                // Resolve the active left/right thresholds for this sample.
                let (lo, hi, calibrating) = if let Some((l, r)) = edges {
                    (l, r, false)
                } else {
                    // Auto: grow the envelope, then relax it gently toward the
                    // center so a one-off outlier doesn't pin an extreme forever.
                    xmin = xmin.min(x);
                    xmax = xmax.max(x);
                    let c = 0.5 * (xmin + xmax);
                    xmin += (c - xmin) * 0.0008;
                    xmax += (c - xmax) * 0.0008;
                    let span = xmax - xmin;
                    if span < MIN_SPAN {
                        (f32::NEG_INFINITY, f32::INFINITY, true) // not ready: no zone
                    } else {
                        (xmin + margin * span, xmax - margin * span, false)
                    }
                };

                // Gate on the physical Super/Meta key, if required.
                let super_held = supers.as_ref().is_none_or(superkey::Watcher::is_down);

                if calibrating {
                    last_left = None;
                    last_right = None;
                    status(format_args!(
                        "   calibrating (x={x:.2}, range {xmin:.2}..{xmax:.2})"
                    ));
                } else if !super_held {
                    // Idle until Super is pressed; reset so re-entry taps right away.
                    last_left = None;
                    last_right = None;
                    status(format_args!("   hold Super  (x={x:.2})"));
                } else if x <= lo {
                    last_right = None;
                    if last_left.is_none_or(|t| now.duration_since(t) >= interval) {
                        kb.tap(uinput::KEY_LEFT).context("tapping Left")?;
                        last_left = Some(now);
                        status(format_args!("<- LEFT  (x={x:.2})"));
                    }
                } else if x >= hi {
                    last_left = None;
                    if last_right.is_none_or(|t| now.duration_since(t) >= interval) {
                        kb.tap(uinput::KEY_RIGHT).context("tapping Right")?;
                        last_right = Some(now);
                        status(format_args!("-> RIGHT (x={x:.2})"));
                    }
                } else {
                    // Center zone: idle, ready to fire on next edge entry.
                    last_left = None;
                    last_right = None;
                    status(format_args!("   center (x={x:.2})"));
                }
            }
            // Frames of streams not subscribed here; unknown tags decode to
            // `None`. `ServerMsg` is `#[non_exhaustive]`, so outside tobii-ipc a
            // match on it needs a wildcard.
            _ => {}
        }
    }
}
