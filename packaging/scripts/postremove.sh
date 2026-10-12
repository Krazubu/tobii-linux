#!/bin/sh
# After tobii-linux is removed, or replaced by another version: either way the
# udev rules and the library may have changed or gone.

ldconfig 2>/dev/null || true
if command -v udevadm > /dev/null 2>&1; then
	udevadm control --reload 2>/dev/null || true
fi
if [ -f /usr/share/icons/hicolor/icon-theme.cache ] \
	&& command -v gtk-update-icon-cache > /dev/null 2>&1; then
	gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
fi
exit 0
