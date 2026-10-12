#!/bin/sh
# Before tobii-linux is unpacked. dpkg passes "install" when it is not
# installed (it never was, or it was removed), "upgrade" over a version still
# installed; rpm the number of versions installed once this one is (1 when
# there was none). Only a fresh install enables the socket (postinstall): an
# upgrade leaves it as the admin left it.

case "$1" in
	install | 1) touch /run/tobii-linux-fresh-install 2> /dev/null || true ;;
esac
exit 0
