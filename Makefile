# Build and install the Tobii Linux stack.
#
#   make build            # cargo build --release
#   make install          # binaries + libtobii.so + headers + udev rules + desktop entry (sudo) + user units
#   make enable           # start the always-on user service now
#   make uninstall        # remove everything installed by `make install`
#   make stage DESTDIR=d  # lay out what the packages carry under d (no sudo, nothing else touched)
#
# Run as your normal user; the system parts invoke sudo themselves. Override
# paths with e.g. `make install PREFIX=/usr`.

PREFIX      ?= /usr/local
BINDIR      ?= $(PREFIX)/bin
LIBDIR      ?= $(PREFIX)/lib
INCLUDEDIR  ?= $(PREFIX)/include
DATADIR     ?= $(PREFIX)/share
DOCDIR      ?= $(DATADIR)/doc/tobii-linux
UDEVDIR     ?= /etc/udev/rules.d
# The dynamic loader does not search /usr/local/lib everywhere (Fedora, for
# one): a Stream Engine client linked with -ltobii would not start. This
# file adds LIBDIR to its search path.
LDCONF      ?= /etc/ld.so.conf.d/tobii.conf
USERUNITDIR ?= $(HOME)/.config/systemd/user
# Where `stage` puts the user units: every user's, not one user's like
# `install`, which puts them in USERUNITDIR.
SYSTEMDUSERDIR ?= $(PREFIX)/lib/systemd/user
SUDO        ?= sudo
# Extra flags for `cargo build`, e.g. --locked.
CARGO_FLAGS ?=

REL         := $(or $(CARGO_TARGET_DIR),target)/release
# tobii5-init-replay is built but NOT installed: every one of its subcommands
# is log analysis, UVC-camera work or diagnostics, none of which the driver
# needs. Run it from $(REL). It is still removed by `uninstall` for anyone who
# installed it with an older Makefile.
BINS        := tobiid tobii-opentrack tobii-gaze-keys tobii-calibrate
LEGACY_BINS := tobii5-init-replay
LIB         := libtobii.so
# The rules that set the uaccess tag must sort before systemd's
# 73-seat-late.rules (`make check-udev`); their old 99- names are removed by
# `install` and `uninstall`.
UDEV_RULES  := 70-tobii-uaccess.rules 70-tobii-uinput.rules 99-tobii-no-uvcvideo.rules
LEGACY_UDEV_RULES := 99-tobii-uaccess.rules 99-tobii-uinput.rules
UNITS       := tobiid.service tobiid.socket tobii-gaze-keys.service
# tobii-calibrate's entry among the desktop's applications, and its icon.
DESKTOP     := packaging/desktop/tobii-calibrate.desktop
ICON        := packaging/desktop/tobii-calibrate.svg
ICONDIR     := $(DATADIR)/icons/hicolor
# The units run the binaries of a build in ~/Work/tobii and the desktop
# entry the one in PATH; installed, both run the ones in BINDIR.
UNIT_SED    = s|^ExecStart=%h/Work/tobii/target/release/|ExecStart=$(BINDIR)/|
DESKTOP_SED = s|^Exec=tobii-calibrate|Exec=$(BINDIR)/tobii-calibrate|
# The packages' documentation, with the licence and notice of the MediaPipe
# models built into tobiid.
DOCS        := README.md INSTALL.md LICENSE
MODEL_DOCS  := crates/tobii-pose/models/LICENSE crates/tobii-pose/models/NOTICE
INCSRC      := crates/tobii-ffi/include
ABI_SMOKE   := crates/tobii-ffi/abi-smoke.c

# The entry points libtobii.so presents to C — exactly the 153 exports of the
# Stream Engine 4.1.0.3 DLL — and the headers that declare them are its whole
# contract with consumers such as OpenTrack's tracker-tobii. `verify-abi`
# checks both against the built library.
ABI_LIST    := crates/tobii-ffi/abi-symbols.txt
ABI_SYMBOLS := $(shell sed 's/ *\#.*//; /^$$/d' $(ABI_LIST))
HEADERS     := $(wildcard $(INCSRC)/tobii/*.h)

# Writes the units into the directory $(1), running the binaries in BINDIR;
# fails on an ExecStart that runs anything else.
define write-units
for u in $(UNITS); do \
	sed '$(UNIT_SED)' systemd/$$u > $(1)/$$u && chmod 0644 $(1)/$$u || exit 1; \
done
if grep -H '^ExecStart=' $(addprefix $(1)/,$(UNITS)) | grep -v ':ExecStart=$(BINDIR)/'; then \
	echo "ExecStart not rewritten to $(BINDIR)"; exit 1; \
fi
endef

.PHONY: build check check-udev check-desktop verify-abi stage install install-bin install-headers install-udev install-desktop install-units enable enable-keys disable uninstall clean

build:
	cargo build --release --workspace $(CARGO_FLAGS)

# Everything CI would run.
check: check-udev check-desktop
	cargo fmt --all --check
	cargo clippy --release --all-targets --workspace -- -D warnings
	cargo test --workspace
	cargo doc --no-deps --workspace

# udev acts on the uaccess tag in systemd's 73-seat-late.rules, so a rule that
# sets it must sort before that file, or the user gets no ACL on the device.
check-udev:
	@for f in systemd/*.rules; do \
		if grep -q 'TAG+="uaccess"' "$$f" \
			&& ! printf '%s\n' "$${f##*/}" 73-seat-late.rules | LC_ALL=C sort -C; then \
			echo "$$f sets uaccess but sorts after 73-seat-late.rules"; exit 1; \
		fi; \
	done

check-desktop:
	@if command -v desktop-file-validate > /dev/null; then desktop-file-validate $(DESKTOP); \
	else echo "desktop-file-validate not found: $(DESKTOP) not checked"; fi

# libtobii.so must keep exporting exactly the listed entry points, and a
# Stream-Engine-shaped C program must still compile against the headers, link
# against the library and get the answers they promise. abi-smoke takes the
# address of every listed symbol through the headers (so each must be declared
# with C linkage and exported) and needs neither the daemon nor a tracker.
verify-abi: build
	@nm -D --defined-only $(REL)/$(LIB) \
		| awk '$$2 == "T" && $$3 ~ /^tobii_/ { print $$3 }' | sort > $(REL)/abi-actual.txt
	@printf '%s\n' $(ABI_SYMBOLS) | sort > $(REL)/abi-expected.txt
	@diff -u $(REL)/abi-expected.txt $(REL)/abi-actual.txt \
		|| { echo "libtobii.so no longer exports exactly $(ABI_LIST)"; exit 1; }
	@echo "libtobii.so exports all $(words $(ABI_SYMBOLS)) listed tobii_* symbols"
	@printf 'X(%s)\n' $(ABI_SYMBOLS) > $(REL)/abi-symbols.inc
	@$(CC) -std=c11 -Wall -Wextra -Werror -I$(INCSRC) -I$(REL) $(ABI_SMOKE) \
		-L$(REL) -ltobii -Wl,-rpath,$(abspath $(REL)) -lm -o $(REL)/abi-smoke
	@$(REL)/abi-smoke

# Every file the packages carry, laid out under DESTDIR as installed under
# PREFIX, and nothing else: no sudo, no reloads, nothing outside DESTDIR.
# The udev rules go to UDEVDIR, the user units to SYSTEMDUSERDIR.
stage: build
	install -Dm 0755 -t $(DESTDIR)$(BINDIR) $(addprefix $(REL)/,$(BINS))
	install -Dm 0644 -t $(DESTDIR)$(LIBDIR) $(REL)/$(LIB)
	install -Dm 0644 -t $(DESTDIR)$(INCLUDEDIR)/tobii $(HEADERS)
	install -Dm 0644 -t $(DESTDIR)$(UDEVDIR) $(addprefix systemd/,$(UDEV_RULES))
	install -d $(DESTDIR)$(SYSTEMDUSERDIR) $(DESTDIR)$(DATADIR)/applications
	$(call write-units,$(DESTDIR)$(SYSTEMDUSERDIR))
	sed '$(DESKTOP_SED)' $(DESKTOP) > $(DESTDIR)$(DATADIR)/applications/$(notdir $(DESKTOP))
	chmod 0644 $(DESTDIR)$(DATADIR)/applications/$(notdir $(DESKTOP))
	install -Dm 0644 -t $(DESTDIR)$(ICONDIR)/scalable/apps $(ICON)
	install -Dm 0644 -t $(DESTDIR)$(DOCDIR) $(DOCS)
	install -Dm 0644 -t $(DESTDIR)$(DOCDIR)/models $(MODEL_DOCS)

install: build install-bin install-headers install-udev install-desktop install-units
	@echo
	@echo "Installed. Start the daemon with:  make enable"
	@echo "(or socket activation:  systemctl --user enable --now tobiid.socket)"

install-bin:
	@if [ "$$(id -u)" = 0 ]; then echo "run as your user, not root (system parts use sudo)"; exit 1; fi
	$(SUDO) install -d $(BINDIR) $(LIBDIR)
	$(SUDO) install -m 0755 $(addprefix $(REL)/,$(BINS)) $(BINDIR)/
	$(SUDO) install -m 0644 $(REL)/$(LIB) $(LIBDIR)/
	echo '$(abspath $(LIBDIR))' | $(SUDO) tee $(LDCONF) > /dev/null
	$(SUDO) ldconfig || true

# The C headers a Stream Engine client compiles against, e.g.
#   cmake .. -DSDK_TOBII=$(PREFIX)   in OpenTrack's tracker-tobii
install-headers:
	$(SUDO) install -d $(INCLUDEDIR)/tobii
	$(SUDO) install -m 0644 $(HEADERS) $(INCLUDEDIR)/tobii/

install-udev:
	$(SUDO) install -m 0644 $(addprefix systemd/,$(UDEV_RULES)) $(UDEVDIR)/
	$(SUDO) rm -f $(addprefix $(UDEVDIR)/,$(LEGACY_UDEV_RULES))
	$(SUDO) udevadm control --reload
	$(SUDO) udevadm trigger
	@echo "udev rules installed — re-plug the tracker if it was already connected"

# Desktops pick a new entry up by themselves. An icon cache, where there is
# one, has to be refreshed or the icon is not found; none is created here,
# since a cache that later installs leave stale would hide their icons.
install-desktop:
	sed '$(DESKTOP_SED)' $(DESKTOP) > $(REL)/$(notdir $(DESKTOP))
	$(SUDO) install -Dm 0644 $(REL)/$(notdir $(DESKTOP)) $(DATADIR)/applications/$(notdir $(DESKTOP))
	$(SUDO) install -Dm 0644 $(ICON) $(ICONDIR)/scalable/apps/$(notdir $(ICON))
	-if [ -f $(ICONDIR)/icon-theme.cache ]; then $(SUDO) gtk-update-icon-cache -q $(ICONDIR); fi

# Install user units, rewriting ExecStart to the installed binary path.
install-units:
	mkdir -p $(USERUNITDIR)
	$(call write-units,$(USERUNITDIR))
	systemctl --user daemon-reload

enable:
	systemctl --user enable --now tobiid.service

# Enable the gaze-to-keyboard binder too (pulls in tobiid via Requires=).
enable-keys:
	systemctl --user enable --now tobii-gaze-keys.service

disable:
	-systemctl --user disable --now tobii-gaze-keys.service
	-systemctl --user disable --now tobiid.service
	-systemctl --user disable --now tobiid.socket

uninstall: disable
	-rm -f $(addprefix $(USERUNITDIR)/,$(UNITS))
	systemctl --user daemon-reload
	$(SUDO) rm -f $(addprefix $(BINDIR)/,$(BINS) $(LEGACY_BINS)) $(LIBDIR)/$(LIB) $(LDCONF)
	$(SUDO) rm -f $(addprefix $(INCLUDEDIR)/tobii/,$(notdir $(HEADERS)))
	-$(SUDO) rmdir $(INCLUDEDIR)/tobii 2>/dev/null
	$(SUDO) rm -f $(addprefix $(UDEVDIR)/,$(UDEV_RULES) $(LEGACY_UDEV_RULES))
	$(SUDO) rm -f $(DATADIR)/applications/$(notdir $(DESKTOP)) $(ICONDIR)/scalable/apps/$(notdir $(ICON))
	-if [ -f $(ICONDIR)/icon-theme.cache ]; then $(SUDO) gtk-update-icon-cache -q $(ICONDIR); fi
	$(SUDO) udevadm control --reload || true
	$(SUDO) ldconfig || true

clean:
	cargo clean
