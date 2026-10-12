#!/bin/sh
# Before tobii-linux is removed or replaced. dpkg passes "remove" when it is
# removed ("upgrade" when a new version replaces it), rpm the number of
# versions left afterwards (0 when it is removed). Only a removal stops the
# socket from coming back at every login.

case "$1" in
	remove | 0) ;;
	*) exit 0 ;;
esac

if command -v systemctl > /dev/null 2>&1; then
	systemctl --global disable tobiid.socket 2>/dev/null || true
fi
exit 0
