#!/bin/sh
# Before tobii-linux is removed or replaced. dpkg passes "remove" when it is
# removed ("upgrade" when a new version replaces it), rpm the number of
# versions left afterwards (0 when it is removed). Only a removal stops the
# socket from coming back at every login.

case "$1" in
	remove | 0) ;;
	*) exit 0 ;;
esac
# The tarball's install under /usr/local, if there is one too, has its own
# tobiid.socket under the same name, and so the same one enablement link:
# leave it to that install.
if [ -f /usr/local/share/tobii-linux/installed-files ]; then
	exit 0
fi

if command -v systemctl > /dev/null 2>&1; then
	systemctl --global disable tobiid.socket 2>/dev/null || true
fi
exit 0
