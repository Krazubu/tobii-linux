# Distribution packaging

## AUR — `tobii-linux-bin`

`PKGBUILD`, `.SRCINFO` and the install hook here are complete and were checked
against the published v0.2.0 tarball: the checksum matches, `package()` runs
clean and lays out 29 files in an Arch-correct tree. They are **not published**;
doing that needs an AUR account and its SSH key:

```sh
git clone ssh://aur@aur.archlinux.org/tobii-linux-bin.git
cp packaging/aur/{PKGBUILD,.SRCINFO,tobii-linux-bin.install} tobii-linux-bin/
cd tobii-linux-bin && git add -A && git commit && git push
```

On each release, `pkgver`, the `source` URL and the `sha256sums` have to follow
in both `PKGBUILD` and `.SRCINFO` — the release's own `SHA256SUMS` carries the
sum. `makepkg --printsrcinfo > .SRCINFO` regenerates the second file if you
have `makepkg` to hand.

**Why `-bin` and not a source package.** `ort-sys` downloads the ONNX Runtime
over the network while it builds, which makepkg's guidelines rule out of
`build()`, and the head-pose model is an embedded asset. A from-source
`tobii-linux` would need onnxruntime vendored or made optional first. The
binary it installs is built in a container by this repository's CI from the
tagged commit, which is the same artifact the `.deb` and `.rpm` come from.

**One thing the package has to fix.** The tarball's systemd units and
`.desktop` hard-code `/usr/local/bin`, where `install.sh` puts the binaries.
An Arch package owns `/usr`, so `package()` rewrites those paths; without that
the units point at files the package does not ship.

## Fedora COPR — not done, and why

COPR builds from source in `mock`, with **no network during the build**, which
the ONNX Runtime download rules out exactly as it does for an AUR source
package. So a COPR recipe is not a matter of writing a `.spec`: it needs the
build made network-free first. Two ways, neither small:

- vendor the ONNX Runtime (or depend on Fedora's `onnxruntime`, and teach `ort`
  to link it) — then a normal `%build`/`%install` spec works and COPR can
  track releases;
- or make the head pose optional behind a Cargo feature, so a COPR build drops
  `tobii-pose` and ships the driver without head tracking. That changes what
  the package *is*, and head pose is half of why people want this.

Until one of those is done, Fedora users have the `.rpm` from the release page,
which `dnf install ./tobii-linux-*.rpm` takes. That is a worse experience than
a repository — no updates — and it is an honest gap rather than a solved
problem.
