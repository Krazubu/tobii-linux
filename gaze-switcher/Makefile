# tobii gaze switcher — a GNOME Shell extension.
UUID := tobii-gaze-switcher@skyggedans.github.io
EXT_DIR ?= $(HOME)/.local/share/gnome-shell/extensions/$(UUID)
FILES := metadata.json extension.js gaze.js gazelib.js

.PHONY: install uninstall test
install:
	install -d "$(EXT_DIR)"
	install -m 644 $(FILES) "$(EXT_DIR)/"
	@# The running shell does not see a new extension until the next login,
	@# so `gnome-extensions enable` would fail: list it in the setting itself.
	@cur=$$(gsettings get org.gnome.shell enabled-extensions); \
	case "$$cur" in \
	  *"'$(UUID)'"*) ;; \
	  "@as []"|"[]") gsettings set org.gnome.shell enabled-extensions "['$(UUID)']" ;; \
	  *) gsettings set org.gnome.shell enabled-extensions "$${cur%]}, '$(UUID)']" ;; \
	esac
	@echo "Installed and enabled. Log out and back in (Wayland) to load it."

uninstall:
	-gnome-extensions disable $(UUID)
	rm -rf "$(EXT_DIR)"
	@echo "Removed tobii gaze switcher."

test:
	gjs -m tests/test_gazelib.js
	gjs -m tests/test_gaze.js
