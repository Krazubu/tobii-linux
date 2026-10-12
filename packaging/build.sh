#!/bin/sh
# Builds the release packages inside the container of packaging/Containerfile
# (`make package` runs it there, as the user who owns the tree): the workspace
# and its ABI check, the files `make stage` lays out, then the .deb and .rpm
# (nfpm) and the tar.gz, with their SHA256SUMS, in dist/.
set -eu
cd "$(dirname "$0")/.."

# What the packages promise to need at most, and INSTALL.md and install.sh
# say: the glibc of Ubuntu 22.04.
GLIBC_MAX=2.35

VERSION=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml)
[ -n "$VERSION" ] || { echo "build.sh: no [workspace.package] version in Cargo.toml" >&2; exit 1; }
export VERSION

# A target directory of its own, never mixed with a build for the host's
# glibc, with the crates and onnxruntime cached inside it between runs.
export CARGO_TARGET_DIR="$PWD/target/package"
export CARGO_HOME="$CARGO_TARGET_DIR/cargo-home"
export XDG_CACHE_HOME="$CARGO_TARGET_DIR/cache"
STAGE=$CARGO_TARGET_DIR/stage
DIST=$PWD/dist
TARDIR=tobii-linux-$VERSION-x86_64
mkdir -p "$CARGO_TARGET_DIR"

# onnxruntime calls glibc 2.38's C23 strto*: see compat/isoc23.c.
cc -O2 -fPIC -ffunction-sections -c packaging/compat/isoc23.c -o "$CARGO_TARGET_DIR/isoc23.o"
export RUSTFLAGS="-C link-arg=$CARGO_TARGET_DIR/isoc23.o"

make build verify-abi check-desktop CARGO_FLAGS=--locked

rm -rf "$STAGE" "$DIST"
# The packages install under /usr, the tarball under /usr/local (its
# install.sh adds what the package managers do: ld.so.conf.d, the reloads).
make stage DESTDIR="$STAGE/pkg" PREFIX=/usr UDEVDIR=/usr/lib/udev/rules.d
make stage DESTDIR="$STAGE/tar/$TARDIR/files" PREFIX=/usr/local UDEVDIR=/etc/udev/rules.d
desktop-file-validate "$STAGE/pkg/usr/share/applications/tobii-calibrate.desktop"

# The newest symbol version of $2 (GLIBC, GLIBCXX) that the ELF file $1 needs.
newest() {
	objdump -T "$1" | sed -n "s/.*($2_\([0-9.]*\)).*/\1/p" | sort -uV | tail -1
}
for f in "$STAGE"/pkg/usr/bin/* "$STAGE"/pkg/usr/lib/libtobii.so; do
	glibc=$(newest "$f" GLIBC)
	if [ -z "$glibc" ]; then
		echo "build.sh: read no glibc version from ${f##*/}: cannot check it" >&2
		exit 1
	fi
	if [ "$(printf '%s\n%s\n' "$glibc" "$GLIBC_MAX" | sort -V | tail -1)" != "$GLIBC_MAX" ]; then
		echo "build.sh: ${f##*/} needs glibc $glibc, newer than $GLIBC_MAX" >&2
		exit 1
	fi
	echo "${f##*/}: glibc $glibc, libstdc++ $(newest "$f" GLIBCXX)"
done

mkdir -p "$DIST"
nfpm package --config packaging/nfpm.yaml --packager deb --target "$DIST/"
nfpm package --config packaging/nfpm.yaml --packager rpm --target "$DIST/"

sed "s/@VERSION@/$VERSION/g; s/@GLIBC_MAX@/$GLIBC_MAX/g" packaging/install.sh \
	> "$STAGE/tar/$TARDIR/install.sh"
chmod 0755 "$STAGE/tar/$TARDIR/install.sh"
sed "s/@VERSION@/$VERSION/g; s/@GLIBC_MAX@/$GLIBC_MAX/g" packaging/README.tarball \
	> "$STAGE/tar/$TARDIR/README"
tar -C "$STAGE/tar" --owner=0 --group=0 --numeric-owner -czf "$DIST/$TARDIR.tar.gz" "$TARDIR"

cd "$DIST"
sha256sum -- *.deb *.rpm *.tar.gz > SHA256SUMS
ls -l
