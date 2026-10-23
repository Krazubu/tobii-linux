# Changelog

What changed for someone using the driver or its library. Dates are the
release's; the full history is in the git log.

This project keeps no compatibility promise of its own below 1.0, but
`libtobii.so` does: it presents exactly the 153 entry points of Tobii's
Stream Engine 4.1.0.3 and is held to them by `make verify-abi` on every
change. Where an answer differs from the Windows DLL's it is deliberate and
listed in the README's "where they differ from Windows".

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
