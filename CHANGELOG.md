# Changelog

What changed for someone using the driver or its library. Dates are the
release's; the full history is in the git log.

This project keeps no compatibility promise of its own below 1.0, but
`libtobii.so` does: it presents exactly the 199 entry points of Tobii's
Stream Engine 4.25.0.3 and is held to them by `make verify-abi` on every
change. Where an answer differs from the Windows DLL's it is deliberate and
listed in the README's "where they differ from Windows".

## Unreleased

### Fixed

- **`tobii_wearable_advanced_data_t` is 216 bytes, not 208.** 4.25 inserts a
  validity and a float before `improve_user_position_hmd`, which the 0.3.0
  headers did not have, so those two flags sat at +0xc8/+0xcc where 4.25 puts
  them at +0xd0/+0xd4. The 0.3.0 layout was 4.1's and no 4.25 DLL produces it.
  Found by reading the second of the device's callback-slot tables, the one
  for PRP compound streams; the record is built at
  0x18008e4e0..0x18008e739 and 0x18008e740..0x18008e999 and bounded by a
  memset of 0xc8 bytes from +0x10. What the new pair means is not in the
  binary — neither builder names anything about itself — so it is declared as
  `unknown_c8_validity` and `unknown_cc`, the way `tobii_gaze_raw_t`'s
  unidentified fields already are. Nothing in this repository builds the
  record (`tobii_wearable_advanced_data_subscribe` is a refusal, as the DLL's
  own availability table refuses the stream), so this only affects code
  compiled against these headers and linked against a real 4.25 DLL. The
  record of the other wearable stream, `tobii_wearable_consumer_data_t` at 120
  bytes, and `tobii_user_position_guide_t` at 40, were checked against the
  same tables in both DLLs and are unchanged.

## 0.3.0 — 2026-10-24

The library now follows Stream Engine **4.25.0.3** instead of 4.1.0.3.
`libtobii.so` exports its whole surface — **199 entry points** against 4.1's
153 — and `tobii_get_api_version` reports 4.25.0.3. Of the 46 names 4.25 adds
(two more are renames), sixteen are implemented and thirty are refusals the
DLL itself gives for a tracker it drives without the Tobii service; the README
lists which. A client built against 4.1 is unaffected: no 4.1 entry point
changed its signature, and the only struct that changed for it was already
wrong (below).

### Entry points added from 4.25

- **The tracker's fault and warning lists** as first-class calls:
  `tobii_get_faults` and `tobii_get_warnings` write the same 512-byte text
  `tobii_get_state_string` gives for `TOBII_STATE_FAULT` and
  `TOBII_STATE_WARNING`, and `tobii_faults_subscribe` /
  `tobii_warnings_subscribe` hand a subscriber the whole list on every change,
  from the notifications the daemon already sent.
- **`tobii_output_frequency_subscribe` / `_unsubscribe`.** 4.25 serves the
  output frequency as a stream beside the getter. The receiver is called once,
  from inside the subscribe, with the frequency the tracker runs at; the ET5
  never changes it, so there is no second call.
- **`tobii_responsive_gaze_point_subscribe` / `_unsubscribe`**, the stream
  4.25 numbers 12: the tracker's own less filtered combined gaze point, which
  it sends beside the one `tobii_gaze_point_subscribe` delivers (stream 0x83
  key 0x20). `tobii_stream_supported` now reports 12 supported.

- **4.25's presence stream**, `tobii_presence_subscribe` / `_unsubscribe`:
  the same status and timestamp `tobii_user_presence_subscribe` delivers, in a
  `tobii_presence_t` instead of two arguments. Both are fed from the one
  presence the daemon sends, so a subscriber to either sees the same changes.
- **Signing**, the four calls that touch no device at all:
  `tobii_signing_sign`, `tobii_signing_sign_with_private_key`,
  `tobii_signing_verify` and `tobii_signing_verify_with_public_key` — SHA-256
  with RSA PKCS#1 v1.5 over a PEM key, the signature in standard base64, as
  the DLL does it in statically linked OpenSSL. They return their own
  `tobii_signing_result_t`, **not** a `tobii_error_t`: its values run 0..16
  and every one of them is also a valid error code, so mapping a result
  through the error table gives wrong semantics silently. See
  `tobii/tobii_signing.h`.

  Three of the DLL's behaviours here are deliberately not reproduced, each
  being a memory error rather than a contract: the 640-byte memset of the
  caller's buffer before anything is read, the decode that runs off a
  640-byte frame when the caller reports a signature of 856 bytes or more,
  and `fopen(NULL, "r")` for a null key path. A null argument is
  `TOBII_SIGNING_NULL_ARGUMENT` and nothing is written.

  This is the library's first dependency beyond the daemon's own crates —
  `rsa`, `sha2` and `base64`, all pure Rust. `libtobii.so` still links only
  libc, libm and libgcc_s, and grew from 524 KB to 782 KB.

### Entry points that exist and refuse

Thirty of 4.25's additions answer `TOBII_ERROR_NOT_SUPPORTED` once their
arguments check out, which is what the DLL answers for a tracker it drives
itself: the wearable image, wearable diagnostics image and wearable entrance
pupil streams, the primary camera image, the 3-D facial landmarks and their
group enumeration, the device log, absolute eye openness, proximity data
collection, the algorithm-debug stream, the ambient light level (whose handler
in 4.25 is itself a stub returning the same), the four remote-camera calls,
the extended lens configuration trio and the two algo-box commands. None of
them invents a record: where the audit could not name a field — the 40-byte
primary-camera record, absolute eye openness, the two words of
`tobii_force_frame_sequence` — nothing is declared for it.

One deliberate difference: `tobii_remote_camera_set` answers
`TOBII_ERROR_INVALID_PARAMETER` for a null path, where the DLL dereferences
it first and ships 3 700 bytes of uninitialised stack after it.

### Renamed, following 4.25

- `tobii_multiple_faces_position_subscribe` / `_unsubscribe` are now
  `tobii_multiple_faces_subscribe` / `_unsubscribe`. Same internal stream 5
  and the same refusal as before; 4.25 dropped `_position_` from the names.

### Two struct layouts corrected

Both were found by reverse-engineering Stream Engine 4.25.0.3 and checking
what it does against 4.1.0.3.

- **`tobii_gaze_data_t` was eight bytes short per eye.**
  `tobii_gaze_data_eye_t` carries a fifth `tobii_validity_t` at offset `0x10`,
  for the normalised eye position that follows it, so the eye block is 80
  bytes and the struct 176, with the right eye at `0x60`. This library
  declared four validities, a 76-byte eye and a 168-byte struct — which is
  what Tobii's own 4.1 documentation describes, but both the 4.1 and the 4.25
  DLL emit the fifth field, so every field past `0x10` was misaligned for a
  program built against the real layout. A program built against this
  library's old header has to be rebuilt.

  The new `eye_position_validity` is the tracker's own answer for that
  position rather than a copy of `gaze_origin_validity`: the daemon now sends
  a validity per eye and `libtobii.so` hands it out. 4.1 duplicates the
  gaze-origin flag into the slot and gives the right eye the left eye's flag;
  this library reports the independent flag, as 4.25 does. A daemon from
  before this release sends neither validity, and the library then falls back
  to `gaze_origin_validity`, which is what 4.1 leaves there; a daemon from
  after it paired with an older library is unaffected, because the two
  validities travel as a tail an older library ignores.

- **`tobii_gaze_raw_t` grew to 248 bytes**, as it is in 4.25: sixteen zeroed
  bytes at `0xe8`, which the DLL's dispatch zeroes along with the word at
  `0xe4`. No named field moved, so only a caller that copies
  `sizeof( tobii_gaze_raw_t )` bytes notices — and such a caller used to read
  sixteen bytes of this library's stack where the DLL guarantees zeros. 4.1's
  record is 232 bytes, so this is the first place where the library follows
  4.25 instead.

## 0.2.0 — 2026-10-23

Three weeks of filling in what the Stream Engine does and this library did
not: 86 commits since 0.1.0. Nothing in the existing API changed shape, so a
program built against 0.1.0 keeps working — but two prototypes were wrong and
are now right (below), and a tracker is answered where it used to be refused.

### Entry points that now work

- **Power save.** `tobii_power_save_activate` and `_deactivate` send the
  tracker the Stream Engine's own commands; it carries them out, and gaze
  keeps coming at full rate. One state shared by every client: the last call
  wins, any client may end it, and the daemon ends it when the client holding
  it disconnects or the daemon is stopped. `TOBII_STATE_POWER_SAVE_ACTIVE`
  follows the tracker's own notifications, and `POWER_SAVE_STATE_CHANGED` is
  delivered.
- **The tracker's extension lists** — `tobii_enumerate_extensions` and
  `_enabled_extensions`, asked of the tracker on every call. An Eye Tracker 5
  has none, so both answer an empty list with no error.
- **The stream catalogue's columns** — `tobii_enumerate_stream_type_columns`
  hands over the names the tracker reports for each stream.
- **The custom stream** — `tobii_custom_stream_subscribe` serves the tracker's
  stream `0x1770` as custom stream 0, subscribed by the Stream Engine's rules.
  Its frames carry no data on an Eye Tracker 5, which is what the tracker
  sends.
- **Image collection's subscribe** — started as the Stream Engine starts it.
  The Eye Tracker 5 refuses the stream (it wants a realm open), so the call
  answers `TOBII_ERROR_INTERNAL`, as the Stream Engine does, and no frame
  arrives.
- **Internal stream support** and **the internal device info**
  (`tobii_get_device_info_internal`: the tracker's platform type).
- **Gaze over HID** — `tobii_get_gaze_hid_enabled` reports the tracker's own
  answer (`false` on an Eye Tracker 5), and
  `tobii_get_combined_gaze_hid_track_box` the track box it reports gaze in.
- **An application's own command** — `tobii_send_custom_command` really sends
  the tracker the Stream Engine's command 5000 and hands its answer to the
  receiver. What a valid payload holds is the application's business; nothing
  in the Stream Engine documents it.
- **Diagnostics, logs, clean IR, the display id and the display info** answer
  exactly what the Stream Engine answers an Eye Tracker 5 — each for its own
  reason, and not all the same code. See the README.

Of the Stream Engine's 153 entry points, five are still unimplemented
refusals: reading and listing the illumination modes, arming and disarming
remote wake (the tracker refuses all four itself, so the answer is right), and
the configuration key, which needs a realm no entry point can open.

### Fixed

- **`tobii_set_display_id` and `tobii_get_display_id` took the wrong type.**
  The display id is a string, not a `uint32_t`; the old prototype would have
  made a real call dereference a small integer as a `char*`. The header now
  declares `tobii_display_id_t` (a 256-byte buffer), as the DLL uses it.
- **`tobii_get_configuration_key`'s second and third parameters were
  swapped.** It takes the 512-byte output second and the key name third.
- The user position guide is the tracker's own field, not the secondary range
  it was read from before, so `tobii_eye_position_normalized` and the research
  output now report what the tracker means.
- A wait for the tracker's answer now reads no longer than the wait has left.
  It read in fixed 200 ms slices for as long as the deadline had not passed,
  so a 500 ms wait could end near 700 ms — which held an open up when the
  tracker left a request unanswered.

### Hardware-verified

Every answer above was checked against a real Eye Tracker 5, not only against
the DLL: the probes and their results are in `fixtures/analysis/` (gitignored,
so they are not in the package). The command-5000 path, the HID reads and the
power-save lifecycle were run end to end through the installed library and
daemon.

### Tools

- `tobii5-init-replay stream-probe` and `state-probe` — safe, whitelisted
  probes of the tracker's streams and state commands, dry-run by default.
- `gaze-switcher/` — a GNOME Shell extension that steers Alt+Tab by gaze, now
  standalone (it no longer needs PaperWM).
- `tobii-gaze-keys` — fewer false winks: an open eye may squint for six
  frames, the other eye waits 1.5 s after a double, and no wink fires while
  the head sits at the edge of the tracker's view.

### Project

- **CI** now runs `make check` (rustfmt, clippy with warnings denied, 1124
  tests, docs) and `make verify-abi` on every push and pull request. Until
  now nothing but a developer's own machine checked a change.
- This changelog.

## 0.1.0 — 2026-10-12

First release: the driver stack as a `.deb`, `.rpm` and tarball, built in a
container and published by CI from a `v*` tag.

- `libtobii.so` — all 153 Stream Engine 4.1.0.3 entry points, so an existing
  Stream Engine client links and runs; the gaze, gaze origin, eye position,
  presence, head pose, gaze data, raw gaze, foveated gaze point and IR image
  streams; 2-D calibration; device info, track box, display area, mounting and
  states; notifications; the device/host clock pair.
- `tobiid` — the daemon that owns the tracker and serves every client.
- `tobii-opentrack` — head pose to OpenTrack over UDP.
- `tobii-gaze-keys` — arrow keys and window switching from gaze and winks.
- `tobii-calibrate` — a fullscreen calibration client.
- udev rules, systemd user units and an installer.
