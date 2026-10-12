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
# The directories it made, deepest first: removed again once empty.
DIRS=$SHARE/created-dirs
SELF=$SHARE/install.sh
LDCONF=/etc/ld.so.conf.d/tobii.conf

die() {
	echo "install.sh: $*" >&2
	exit 1
}

usage() {
	echo "usage: sudo $0 [--uninstall]"
}

# Each step after the files are in place or gone may fail: there may be no
# udev or systemd to tell. $1 is "install" after an install.
reload() {
	ldconfig 2> /dev/null || true
	if command -v udevadm > /dev/null 2>&1; then
		udevadm control --reload 2> /dev/null || true
		udevadm trigger --subsystem-match=usb --attr-match=idVendor=2104 2> /dev/null || true
		# uinput is often a module nothing has loaded yet, whose static node
		# stays root's, rule or not, until a reboot: loaded now, its add event
		# applies the rule (the trigger covers a uinput built into the kernel).
		if [ "${1:-}" = install ]; then
			modprobe uinput 2> /dev/null || true
		fi
		udevadm trigger --subsystem-match=misc --sysname-match=uinput 2> /dev/null || true
	fi
	if [ -f /usr/local/share/icons/hicolor/icon-theme.cache ] \
		&& command -v gtk-update-icon-cache > /dev/null 2>&1; then
		gtk-update-icon-cache -q /usr/local/share/icons/hicolor 2> /dev/null || true
	fi
}

# Root is about to delete what the lists name: only lists that only root
# could have written are followed, and only to paths this installs under.
check_manifest() {
	for p in "$SHARE" "$MANIFEST" "$DIRS"; do
		[ -e "$p" ] || continue
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
	if [ -f "$DIRS" ]; then
		while IFS= read -r d; do
			if installable "$d"; then
				rmdir -- "$d" 2> /dev/null || true
			fi
		done < "$DIRS"
	fi
}

# "deb" or "rpm" when tobii-linux is installed as that package, else nothing.
package_installed() {
	if command -v dpkg-query > /dev/null 2>&1 \
		&& dpkg-query -W -f '${Status}' tobii-linux 2> /dev/null | grep -q 'ok installed'; then
		echo deb
	elif command -v rpm > /dev/null 2>&1 && rpm -q tobii-linux > /dev/null 2>&1; then
		echo rpm
	fi
}

uninstall() {
	[ -f "$MANIFEST" ] || die "nothing to remove: $MANIFEST is missing"
	# A package installed too has its own tobiid.socket under the same name,
	# and so the same one enablement link: a disable would take it from the
	# package. Left as it is, the link is pointed at the package's unit once
	# this one is gone.
	pkg=$(package_installed)
	enabled=
	if command -v systemctl > /dev/null 2>&1; then
		if [ -z "$pkg" ]; then
			systemctl --global disable tobiid.socket 2> /dev/null || true
		elif systemctl --global is-enabled tobiid.socket > /dev/null 2>&1; then
			enabled=yes
		fi
	fi
	remove_files
	rm -f -- "$MANIFEST" "$DIRS" "$SELF"
	rmdir -- "$SHARE" 2> /dev/null || true
	reload
	if [ -n "$enabled" ]; then
		systemctl --global reenable tobiid.socket 2> /dev/null || true
	fi
	echo "tobii-linux removed. A daemon still running stops at logout, or now with"
	echo "  systemctl --user stop 'tobii*'; pkill -x tobiid"
}

install_files() {
	here=$(cd "$(dirname "$0")" && pwd)
	[ -d "$here/files" ] || die "no files/ next to $0: run the install.sh of the unpacked tarball"

	glibc=$(getconf GNU_LIBC_VERSION 2> /dev/null | cut -d' ' -f2) || true
	[ -n "$glibc" ] || die "no glibc here (musl?): the binaries need glibc $GLIBC_MIN or newer"
	[ "$(printf '%s\n%s\n' "$GLIBC_MIN" "$glibc" | sort -V | head -1)" = "$GLIBC_MIN" ] \
		|| die "glibc $glibc is older than the $GLIBC_MIN the binaries need"
	# What else the binaries load: libusb, libudev, a libstdc++ from GCC 12 or
	# newer. On a noexec mount ldd can map nothing and says "not a dynamic
	# executable" instead: then the installed copies are checked, below.
	checked=$(cd "$here/files" && ldd usr/local/bin/* usr/local/lib/libtobii.so 2>&1 || true)
	missing=$(printf '%s\n' "$checked" | grep 'not found' || true)
	[ -z "$missing" ] || die "the binaries cannot find:
$missing
install libusb-1.0 and libudev (and a newer libstdc++ if it is named), then run this again"
	recheck=
	if printf '%s\n' "$checked" | grep -q 'not a dynamic executable'; then
		recheck=yes
	fi

	pkg=$(package_installed)
	[ -z "$pkg" ] || die "tobii-linux is installed as a package ($pkg): upgrade or remove that instead"

	list=$(mktemp)
	dirs=$(mktemp)
	trap 'rm -f "$list" "$dirs"' EXIT
	(cd "$here/files" && find . -type f | sed 's|^\./|/|' | sort) > "$list"
	# Every path before anything is touched.
	while IFS= read -r f; do
		installable "$f" || die "the tarball holds $f, outside what it may install"
	done < "$list"

	fresh=yes
	if [ -f "$MANIFEST" ]; then
		echo "replacing the tobii-linux installed before"
		remove_files
		fresh=
	fi

	while IFS= read -r f; do
		d=${f%/*}
		while [ -n "$d" ] && [ ! -d "$d" ]; do
			echo "$d"
			d=${d%/*}
		done
	done < "$list" | sort -ru > "$dirs"
	# File by file, owned by root: the unpacked tree belongs to whoever
	# unpacked it, and udev runs what its rules say as root. The modes go by
	# where each file goes, the programs 0755 and the rest 0644, not by the
	# tree, whose bits the unpacking user's umask, a noexec mount (where even
	# root may not execute) or a filesystem without Unix modes (FAT) decide.
	while IFS= read -r f; do
		case $f in
			/usr/local/bin/*) mode=0755 ;;
			*) mode=0644 ;;
		esac
		install -D -o root -g root -m "$mode" "$here/files$f" "$f"
	done < "$list"
	# The loader does not search /usr/local/lib everywhere (Fedora, for one).
	echo /usr/local/lib > "$LDCONF"
	chmod 0644 "$LDCONF"
	echo "$LDCONF" >> "$list"

	install -d -m 0755 "$SHARE"
	install -m 0644 "$list" "$MANIFEST"
	install -m 0644 "$dirs" "$DIRS"
	install -m 0755 "$here/$(basename "$0")" "$SELF"
	if command -v restorecon > /dev/null 2>&1; then
		restorecon -F -f "$MANIFEST" 2> /dev/null || true
	fi

	reload install
	# A first install enables the socket for every user; one over an earlier
	# install leaves it enabled or disabled, as it was.
	if [ -n "$fresh" ] && command -v systemctl > /dev/null 2>&1; then
		systemctl --global enable tobiid.socket 2> /dev/null || true
	fi

	if [ -n "$recheck" ]; then
		missing=$(grep -E '^/usr/local/(bin/|lib/libtobii\.so$)' "$MANIFEST" \
			| while IFS= read -r f; do ldd "$f" 2>&1; done | grep 'not found' || true)
		if [ -n "$missing" ]; then
			cat >&2 << EOF
install.sh: installed, but the binaries cannot find (the tree was on a noexec
mount, where they could not be checked before):
$missing
Install libusb-1.0 and libudev (and a newer libstdc++ if it is named).
EOF
		fi
	fi

	cat << EOF
tobii-linux $VERSION installed under /usr/local.
  - re-plug the tracker if it is connected;
  - tobiid starts on demand from your next login, or now after
      systemctl --user start tobiid.socket
    ("Tobii Calibration" starts it by itself either way);
  - gaze to arrow keys: join the input group, which lets every program you
    run read all keyboards (INSTALL.md §3), and reboot,
      sudo usermod -aG input "\$USER"
    then:  systemctl --user enable --now tobii-gaze-keys.service
  - head pose to OpenTrack (its "UDP over network" input, port 4242):
      systemctl --user start tobii-opentrack     (enable --now: at every login)
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
