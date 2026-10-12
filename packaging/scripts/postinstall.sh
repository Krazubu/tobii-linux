#!/bin/sh
# After tobii-linux is installed or upgraded. dpkg passes "configure", rpm
# the number of versions now installed; either way the same steps follow.
# None of them may fail the install: in a container or a chroot there is no
# udev or systemd to talk to.

ldconfig 2>/dev/null || true

if command -v udevadm > /dev/null 2>&1; then
	udevadm control --reload 2>/dev/null || true
	# The tracker and /dev/uinput as they are now, so that the rules apply
	# without re-plugging.
	udevadm trigger --subsystem-match=usb --attr-match=idVendor=2104 2>/dev/null || true
	udevadm trigger --subsystem-match=misc --sysname-match=uinput 2>/dev/null || true
fi

# A fresh install (as preinstall saw it, or as dpkg configuring no earlier
# version or rpm installing the first says) enables the socket for every
# user: from their next login, a client's first connect starts tobiid. An
# upgrade leaves it enabled or disabled, as it was.
fresh=
if [ -e /run/tobii-linux-fresh-install ]; then
	fresh=yes
	rm -f /run/tobii-linux-fresh-install
fi
case "$1" in
	configure) [ -n "${2:-}" ] || fresh=yes ;;
	1) fresh=yes ;;
esac
if [ -n "$fresh" ] && command -v systemctl > /dev/null 2>&1; then
	systemctl --global enable tobiid.socket 2>/dev/null || true
fi

# Desktops pick the entry up by themselves; an icon cache must be refreshed.
if [ -f /usr/share/icons/hicolor/icon-theme.cache ] \
	&& command -v gtk-update-icon-cache > /dev/null 2>&1; then
	gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
fi

cat << 'EOF'

tobii-linux:
  - re-plug the tracker if it is connected;
  - tobiid starts on demand from your next login, or now after
      systemctl --user start tobiid.socket
    ("Tobii Calibration" starts it by itself either way);
  - gaze to arrow keys: join the input group and log in again,
      sudo usermod -aG input "$USER"
    then:  systemctl --user enable --now tobii-gaze-keys.service
  - after an upgrade, restart a running daemon:  systemctl --user restart tobiid

EOF
if [ -f /usr/local/share/tobii-linux/installed-files ]; then
	cat << 'EOF'
warning: tobii-linux is installed from its tarball too, under /usr/local,
  whose programs and units come first; remove that one:
    sudo /usr/local/share/tobii-linux/install.sh --uninstall

EOF
fi
exit 0
