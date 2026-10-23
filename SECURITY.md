# Security

## Reporting a vulnerability

Use GitHub's private reporting — **Security → Report a vulnerability** on
this repository. That opens an advisory only the maintainer can see. Please do
not open a public issue for something exploitable.

Expect a first reply within a week. This is a one-person project with no
release cadence, so a fix lands when it is written and is published as the
next tagged release; if the problem is serious I will say so in the advisory
and tag sooner.

## What this software actually is

Worth knowing before you weigh a report:

- **Nothing runs as root.** `tobiid` and the clients are systemd *user*
  units. The installer needs root to put files in place and to install the
  udev rules, and nothing else does.
- **The tracker is reached over libusb**, not through a kernel driver of ours.
  The udev rule tags the device `uaccess`, so the user logged in at the seat
  gets it — the same way a webcam or a FIDO key is handed over. There is no
  setuid binary and no privileged helper.
- **The daemon's socket is per-user**, at `$XDG_RUNTIME_DIR/tobiid.sock`, in a
  directory only that user may enter. Any process of that user can therefore
  talk to the daemon and read its gaze stream. That is the intended model, and
  the boundary is the user account, not the process.
- **`uinput` is a real escalation surface and is opt-in.**
  `tobii-gaze-keys` synthesises key presses, which needs membership of the
  `input` group (the rule sets `GROUP="input", MODE="0660"`). Anyone in that
  group can inject input into the session — that is a property of `uinput`,
  not of this project, and it is why the install does not add you to the group
  and the documentation tells you to decide for yourself.

## What gaze data is

Gaze is biometric-adjacent: where someone looks says a good deal about them.
This driver keeps it local — nothing is sent anywhere, there is no telemetry
and no network listener. What is written to disk is the user's calibration
(`~/.config/tobii/`) and, only if a research tool is run explicitly, logs
under paths the user names.

If you find a path by which gaze samples, the IR image stream or the
calibration leave the machine or reach another user, that is a vulnerability
and I want to hear about it.

## Things that are known and are not vulnerabilities

- The tracker's **serial number** is in this repository's early git history,
  inside a USB capture committed in the first release and removed later. It is
  the owner's own device and the decision to leave it is theirs. Current code,
  logs and packages are checked for it on every release
  (`packaging/scan-serial.py`).
- `libtobii.so` reproduces the Stream Engine's API faithfully, including that
  it **checks no licence**. A program can call entry points the Windows SDK
  reserves for a higher feature group. That is deliberate and documented in
  the README; the tracker is the owner's and there is no licence server to
  defer to.
- `tobii_send_custom_command` passes an application's bytes to the tracker's
  own command channel. Any process of the user can therefore send the tracker
  arbitrary vendor commands — again the account boundary, and the same thing
  the vendor's own SDK offers.
- The research tools under `tobii5-init-replay` can write to the tracker. They
  are dry-run by default and refuse anything outside a closed whitelist, but
  they are developer tools and are not installed by the packages.

## Supported versions

The latest tagged release, and `master`. There are no maintained branches
behind them.
