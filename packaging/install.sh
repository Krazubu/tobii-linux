#!/bin/sh
# Installs tobii-linux @VERSION@ from this unpacked tarball under /usr/local,
# where no package manager puts files, and removes it again:
#
#     sudo ./install.sh
#     sudo /usr/local/share/tobii-linux/install.sh --uninstall
#
# Running it again over an earlier install replaces that one. On Debian,
# Ubuntu, Fedora or openSUSE the .deb or .rpm installs the same, and lets the
# package manager upgrade and remove it.
set -eu
umask 022

VERSION=@VERSION@
# The binaries were built against this glibc and need it, or a newer one.
GLIBC_MIN=@GLIBC_MAX@
SHARE=/usr/local/share/tobii-linux
MANIFEST=$SHARE/installed-files
SELF=$SHARE/install.sh
LDCONF=/etc/ld.so.conf.d/tobii.conf
# The directories only this creates, removed once empty.
OWN_DIRS="/usr/local/include/tobii /usr/local/share/doc/tobii-linux/models /usr/local/share/doc/tobii-linux"

die() {
	echo "install.sh: $*" >&2
	exit 1
}

usage() {
	echo "usage: sudo $0 [--uninstall]"
}

# Each step after the files are in place or gone may fail: there may be no
# udev or systemd to tell.
reload() {
	ldconfig 2> /dev/null || true
	if command -v udevadm > /dev/null 2>&1; then
		udevadm control --reload 2> /dev/null || true
		udevadm trigger --subsystem-match=usb --attr-match=idVendor=2104 2> /dev/null || true
		udevadm trigger --subsystem-match=misc --sysname-match=uinput 2> /dev/null || true
	fi
	if [ -f /usr/local/share/icons/hicolor/icon-theme.cache ] \
		&& command -v gtk-update-icon-cache > /dev/null 2>&1; then
		gtk-update-icon-cache -q /usr/local/share/icons/hicolor 2> /dev/null || true
	fi
}

# Root is about to delete what the list names: only a list that only root
# could have written is followed, and only to paths this installs under.
check_manifest() {
	for p in "$SHARE" "$MANIFEST"; do
		[ "$(stat -c %u "$p")" = 0 ] || die "$p is not root's: not following it"
		case $(stat -c %A "$p") in
			?????w???? | ????????w?) die "$p is writable by others than root: not following it" ;;
		esac
	done
}

installable() {
	case $1 in
		*/../* | */./* | */..) return 1 ;;
		/usr/local/?* | /etc/udev/rules.d/?* | "$LDCONF") return 0 ;;
		*) return 1 ;;
	esac
}

remove_files() {
	check_manifest
	while IFS= read -r f; do
		if ! installable "$f"; then
			echo "install.sh: not removing $f: not a path this installs" >&2
			continue
		fi
		if [ -f "$f" ] || [ -L "$f" ]; then
			rm -f -- "$f"
		fi
	done < "$MANIFEST"
	for d in $OWN_DIRS; do
		rmdir -- "$d" 2> /dev/null || true
	done
}

uninstall() {
	[ -f "$MANIFEST" ] || die "nothing to remove: $MANIFEST is missing"
	if command -v systemctl > /dev/null 2>&1; then
		systemctl --global disable tobiid.socket 2> /dev/null || true
	fi
	remove_files
	rm -f -- "$MANIFEST" "$SELF"
	rmdir -- "$SHARE" 2> /dev/null || true
	reload
	echo "tobii-linux removed. A daemon still running stops at logout, or now with"
	echo "  systemctl --user stop tobiid.socket tobiid.service"
}

install_files() {
	here=$(cd "$(dirname "$0")" && pwd)
	[ -d "$here/files" ] || die "no files/ next to $0: run the install.sh of the unpacked tarball"

	glibc=$(getconf GNU_LIBC_VERSION 2> /dev/null | cut -d' ' -f2) || true
	[ -n "$glibc" ] || die "no glibc here (musl?): the binaries need glibc $GLIBC_MIN or newer"
	[ "$(printf '%s\n%s\n' "$GLIBC_MIN" "$glibc" | sort -V | head -1)" = "$GLIBC_MIN" ] \
		|| die "glibc $glibc is older than the $GLIBC_MIN the binaries need"
	# What else the binaries load: libusb, libudev, a libstdc++ from GCC 12 or newer.
	missing=$(cd "$here/files" && ldd usr/local/bin/* usr/local/lib/libtobii.so 2>&1 | grep 'not found' || true)
	[ -z "$missing" ] || die "the binaries cannot find:
$missing
install libusb-1.0 and libudev (and a newer libstdc++ if it is named), then run this again"

	if command -v dpkg-query > /dev/null 2>&1 \
		&& dpkg-query -W -f '${Status}' tobii-linux 2> /dev/null | grep -q 'ok installed'; then
		die "tobii-linux is installed as a .deb: upgrade or remove that instead"
	fi
	if command -v rpm > /dev/null 2>&1 && rpm -q tobii-linux > /dev/null 2>&1; then
		die "tobii-linux is installed as an .rpm: upgrade or remove that instead"
	fi

	if [ -f "$MANIFEST" ]; then
		echo "replacing the tobii-linux installed before"
		remove_files
	fi

	list=$(mktemp)
	trap 'rm -f "$list"' EXIT
	(cd "$here/files" && find . -type f | sed 's|^\./|/|' | sort) > "$list"
	# File by file, owned by root: the unpacked tree belongs to whoever
	# unpacked it, and udev runs what its rules say as root.
	while IFS= read -r f; do
		installable "$f" || die "the tarball holds $f, outside what it may install"
		install -D -o root -g root -m "$(stat -c %a "$here/files$f")" "$here/files$f" "$f"
	done < "$list"
	# The loader does not search /usr/local/lib everywhere (Fedora, for one).
	echo /usr/local/lib > "$LDCONF"
	chmod 0644 "$LDCONF"
	echo "$LDCONF" >> "$list"

	install -d -m 0755 "$SHARE"
	install -m 0644 "$list" "$MANIFEST"
	install -m 0755 "$here/$(basename "$0")" "$SELF"
	if command -v restorecon > /dev/null 2>&1; then
		restorecon -F -f "$MANIFEST" 2> /dev/null || true
	fi

	reload
	if command -v systemctl > /dev/null 2>&1; then
		systemctl --global enable tobiid.socket 2> /dev/null || true
	fi

	cat << EOF
tobii-linux $VERSION installed under /usr/local.
  - re-plug the tracker if it is connected;
  - tobiid starts on demand from your next login, or now after
      systemctl --user start tobiid.socket
    ("Tobii Calibration" starts it by itself either way);
  - gaze to arrow keys: join the input group and log in again,
      sudo usermod -aG input "\$USER"
    then:  systemctl --user enable --now tobii-gaze-keys.service
  - after an upgrade, restart a running daemon:  systemctl --user restart tobiid
  - to remove it:  sudo $SELF --uninstall
EOF
}

case ${1:-} in
	"") action=install_files ;;
	--uninstall) action=uninstall ;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		usage >&2
		exit 2
		;;
esac
[ "$(id -u)" = 0 ] || die "run it as root: sudo $0 ${1:-}"
[ "$(uname -m)" = x86_64 ] || die "the binaries are for x86_64, not $(uname -m)"
$action
