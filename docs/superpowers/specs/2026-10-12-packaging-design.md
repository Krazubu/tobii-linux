# Release packages (.deb, .rpm, tar.gz) — design

**Date:** 2026-10-12
**Status:** approved (design and plan)
**Author:** brainstormed with Claude

## Goal

Install the driver on any Linux without building it: the daemon `tobiid`,
`libtobii.so`, `tobii-calibrate`, `tobii-opentrack`, `tobii-gaze-keys` and the
C headers, from one package, with `tobii-calibrate` showing up among the
desktop's applications with its own icon.

Until now the only way in was `make install` from a source tree, on the
machine that built it.

## Findings (verified on the author's Fedora 44 machine)

1. **The binaries do not travel.** `target/release` needs glibc 2.43
   (`asinf@GLIBC_2.43`, `pidfd_spawnp@GLIBC_2.39`, …): it will not start on
   Ubuntu 22.04/24.04 or Debian 12. Those versions are only what the linker
   picked on a new system, so building on an old one fixes it. `tobiid` is
   otherwise self-contained: both ONNX models and the init capture are
   embedded (`include_bytes!`/`include_str!`) and onnxruntime is linked
   statically. Outside it needs only `libusb-1.0`, `libudev` and `libstdc++`
   (GLIBCXX_3.4.30, i.e. GCC ≥ 12). `libtobii.so`, `tobii-calibrate` and the
   clients need only libc; `tobii-calibrate` dlopens Wayland and X11.
2. **The uaccess rule never applied.** The tracker's node had no ACL for the
   logged-in user although udev tagged it `uaccess`: the tag only takes effect
   in rules that sort before `73-seat-late.rules`, and ours was `99-`. Access
   came from a stray local rule with `MODE="0666"` that is not in the
   repository. On a clean system `tobiid` could not open the device.
3. **The calibration window had no app_id / WM_CLASS**, only a title, so no
   desktop could match it to a `.desktop` entry.
4. **Clients start the daemon themselves:** `tobii_ipc::connect_or_spawn`
   looks for `tobiid` next to the client, then in `PATH`, so an installed
   `tobii-calibrate` works without systemd. The daemon also supports socket
   activation (`LISTEN_FDS`).

## Decisions

- Formats: a tar.gz with `install.sh` for any distribution, plus `.deb` and
  `.rpm` for clean upgrades and removal through the package manager.
  AppImage and Flatpak cannot carry udev rules, a system library for
  OpenTrack or systemd units.
- Contents: all five binaries, `libtobii.so`, the headers.
- Start: `tobiid.socket` enabled globally (`systemctl --global`): the daemon
  starts on a client's first connect, and the IR illuminator stays off until
  something needs the tracker.
- Build: `make package` locally (podman) and a GitHub Actions workflow on
  `v*` tags that builds the same way and publishes a GitHub Release.

## Design

### One layout: `make stage`

`make stage DESTDIR=… PREFIX=…` copies every installed file into `DESTDIR` and
touches nothing on the system (no sudo, no udev reload, no systemctl). The
packages are built from it. The lists of what is installed live in Makefile
variables shared by `stage`, `install` and `uninstall`, so they cannot drift.
`make install` keeps its behaviour (user units in `~/.config/systemd/user`)
and gains the desktop entry and icon.

### Build: an Ubuntu 22.04 container

glibc 2.35 and libstdc++ from GCC 12. `packaging/Containerfile` holds the
toolchain (pinned Rust, pinned nfpm with a checked sha256);
`packaging/build.sh` runs inside it, both locally and in CI:
`make build verify-abi` into a target directory of its own
(`target/package`, never mixed with the host's glibc-2.43 build), then
`make stage` for each layout, nfpm for `.deb` and `.rpm`, the tar.gz, and
`SHA256SUMS`, all in `dist/`.

If the onnxruntime that ort downloads refers to glibc 2.38's
`__isoc23_strto*` (the host build has them, from somewhere), a container-only
shim maps them to `strto*`; if libstdc++ is short too, the baseline moves to
Ubuntu 24.04.

### Layout

| | .deb | .rpm | tar.gz (`/usr/local`) |
|---|---|---|---|
| binaries | `/usr/bin` | `/usr/bin` | `/usr/local/bin` |
| `libtobii.so` | `/usr/lib/x86_64-linux-gnu` | `/usr/lib64` | `/usr/local/lib` + `/etc/ld.so.conf.d/tobii.conf` |
| headers | `/usr/include/tobii` | same | `/usr/local/include/tobii` |
| udev rules | `/usr/lib/udev/rules.d` | same | `/etc/udev/rules.d` |
| user units | `/usr/lib/systemd/user` | same | `/usr/local/lib/systemd/user` |
| desktop entry, icon | `/usr/share/applications`, `/usr/share/icons/hicolor/scalable/apps` | same | same under `/usr/local/share` |
| docs and licences | `/usr/share/doc/tobii-linux/` | same | `/usr/local/share/doc/tobii-linux/` |

The docs include the Apache-2.0 LICENSE and NOTICE of the MediaPipe models.

Dependencies: deb `libusb-1.0-0, libudev1, libstdc++6 (>= 12)`, recommends
`libwayland-client0, libxkbcommon0`; rpm by soname
(`libusb-1.0.so.0()(64bit)`, `libudev.so.1()(64bit)`,
`libstdc++.so.6()(64bit)`), which resolves on Fedora and openSUSE alike.

### Install and removal scripts

Shared by deb and rpm (they tell `configure/remove/upgrade` from `1/0`), every
step `|| true` so that a container or chroot install does not fail:

- after install: `ldconfig`; udev reload and a trigger for the tracker and
  for `/dev/uinput`; `systemctl --global enable tobiid.socket`; the desktop
  database and icon cache when their tools exist; a short note (re-plug the
  tracker; the socket listens from the next login or after
  `systemctl --user start tobiid.socket`, and `tobii-calibrate` starts the
  daemon anyway; how to enable gaze-keys; restart `tobiid` after an upgrade);
- before removal (not on upgrade): `systemctl --global disable tobiid.socket`;
- after removal: udev reload, `ldconfig`.

`install.sh` (as root) checks for x86_64, glibc ≥ 2.35, `libusb-1.0.so.0` and
that no `tobii-linux` package is installed, copies its `files/` tree onto `/`,
records what it installed in `/usr/local/share/tobii-linux/installed-files`
(with a copy of itself next to it), adds `/etc/ld.so.conf.d/tobii.conf` and
runs the same steps. `install.sh --uninstall` removes what it recorded.

### Fixes the packages need

- udev: `70-tobii-uaccess.rules` (was `99-`); the uinput rule becomes
  `70-tobii-uinput.rules`. `make uninstall` removes the old names too.
  (The design also gave `/dev/uinput` the uaccess tag, so that
  `tobii-gaze-keys` would run without joining `input`. Review found that its
  default mode reads the keyboards, `/dev/input/event*`, to see Super held,
  which needs the group anyway, so the tag bought nothing. The rule keeps
  `GROUP="input"` alone, and the docs say what membership grants: every
  program of its members reads every keyboard and can inject input.)
- `tobii-calibrate` names its window `tobii-calibrate` (Wayland app_id and
  X11 WM_CLASS), matching `StartupWMClass` in its desktop entry.
- The units' `Documentation=` points at the repository (systemd rejected the
  old value with a warning).

### Desktop entry and icon

`packaging/desktop/tobii-calibrate.desktop` ("Tobii Calibration",
`Name[ru]`, `Settings;HardwareSettings;`, keywords, `StartupWMClass`) with an
action that skips the display setup (it is needed once), and a hand-drawn SVG
icon: an eye with a calibration target for a pupil.

### CI

`.github/workflows/release.yml`, on `v*` tags and by hand, on
`ubuntu-24.04`: checkout (pinned SHA), `make package CONTAINER=docker`. On a
tag it fails unless the tag is `v` + the workspace version, then
`gh release create` with the artifacts (`contents: write`, no third-party
actions); by hand it uploads them as workflow artifacts.

## Supported systems

With the 22.04 baseline: Ubuntu and Mint 22.04+, Debian 12+, Fedora 36+,
openSUSE Tumbleweed, Arch (tar.gz), RHEL/Alma/Rocky 10. Not RHEL 9
(libstdc++ from GCC 11). x86_64 only.

## Verification

1. `make check` and `make verify-abi` on the host.
2. `make package` gives the three artifacts and `SHA256SUMS`; no binary needs
   more than GLIBC 2.35.
3. Test installs in clean containers (systemd installed first, for
   `systemctl`): `.deb` on `ubuntu:22.04` and `debian:12`, `.rpm` on
   `fedora:44`, tar.gz on `archlinux` with `--uninstall`. Every file in place;
   `ldd` resolves everything; `--help` runs for the three clients (`tobiid`
   has no options: `ldd` only); `ldconfig -p` finds `libtobii`; the
   `sockets.target.wants` link exists; removal leaves nothing behind.
4. On the desktop: `desktop-file-validate`; the entry appears in the
   application grid; a `--dry-run --windowed` window carries the icon in the
   dock and Alt+Tab; started from the menu without a daemon, the window shows
   the error.
5. uaccess on the live tracker, with the stray `MODE="0666"` rule moved
   aside: `getfacl` shows the user's ACL and `tobiid` opens the device.
6. Before anything is published: the artifacts scanned for the tracker's
   serial (ASCII, hex, UTF-16LE).
