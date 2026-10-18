# tobii gaze switcher

A GNOME Shell extension: while the Alt+Tab switcher is open, the tile you
look at becomes the selection; release Alt to activate it (the switcher
activates the selected window itself).

- Gaze selects a tile once the popup shows (0.15 s in), so a quick Alt+Tab
  still goes to the previous window.
- Tab and the arrow keys still move the selection; gaze takes over again
  once you look at another tile.
- Any GNOME switcher popup works (windows or applications), on the primary
  monitor, where GNOME shows it.

## Requirements
- GNOME Shell 50 on Wayland.
- `tobiid` running, as for `tobii-gaze-keys`.
- `gjs` (for the tests).

## Install
    make -C gaze-switcher install
    # then log out and back in (Wayland loads new extensions at login)

`install` copies the extension to
`~/.local/share/gnome-shell/extensions/tobii-gaze-switcher@skyggedans.github.io`
and enables it.

## Uninstall
    make -C gaze-switcher uninstall

## Test
    make -C gaze-switcher test                    # unit tests (no shell, no tobiid)
    gjs -m gaze-switcher/tests/probe_socket.js     # live socket probe (needs tobiid)

## Tuning (constants at the top of gaze.js)
- `EMA_ALPHA` (0..1): gaze smoothing while you fixate. Higher = snappier but
  jumpier. Default 0.4.
- `JUMP_PX`: a move farther than this is a saccade, and the selection follows
  it on the next sample instead of the smoothing crawling over. Keep it above
  the gaze jitter (about 10 px from sample to sample) and below a tile's
  width. Default 48.
- `HYSTERESIS_PX`: border deadband before switching tiles; raise if selection
  flickers between adjacent tiles. Default 24.
- `RECONNECT_MS`: backoff between reconnect attempts to tobiid. Default 1000.

## Troubleshooting
- Watch logs: `journalctl --user -b 0 -f | grep '#tobii-gaze'`.
- Check it is loaded: `gnome-extensions info tobii-gaze-switcher@skyggedans.github.io`
  shows `State: ACTIVE`.
- No tile selected while the switcher is open → check that tobiid runs and
  sends gaze: `gjs -m gaze-switcher/tests/probe_socket.js` prints
  `subscribed ok=true`, then ten `gaze valid=…` lines.
