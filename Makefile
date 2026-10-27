# omarchy-plugins — install each plugin into the Omarchy shell
#
#   make install                 install everything
#   make install-omaclock        one at a time (omaclock, omadesklabel, omadock, omamail, omafeed)
#   make uninstall-omaclock      unlink and drop it from shell.json
#
# Shell plugins are symlinked from here into ~/.config/omarchy/plugins/, so
# edits in this repo reload live. omafeed is a standalone binary and uses its
# own Makefile.

PLUGIN_DIR := $(HOME)/.config/omarchy/plugins
AUTHOR     := iluxav
PLUGINS    := omaclock omadesklabel omadock omamail

# Where each widget goes in the bar (omadock is a service, so it has none).
PLACE_omaclock     := --after omarchy.clock
PLACE_omadesklabel := --section left --after omarchy.menu
PLACE_omadock      :=
PLACE_omamail      := --after omarchy.system-update

.PHONY: install uninstall $(addprefix install-,$(PLUGINS) omafeed) $(addprefix uninstall-,$(PLUGINS) omafeed)

install: $(addprefix install-,$(PLUGINS) omafeed)

uninstall: $(addprefix uninstall-,$(PLUGINS) omafeed)

# The shell picks up a new folder on rescan; retry enable briefly in case the
# rescan has not finished yet.
$(addprefix install-,$(PLUGINS)): install-%:
	@mkdir -p $(PLUGIN_DIR)
	@if [ -e $(PLUGIN_DIR)/$(AUTHOR).$* ] && [ ! -L $(PLUGIN_DIR)/$(AUTHOR).$* ]; then \
		echo "$(PLUGIN_DIR)/$(AUTHOR).$* exists and is not a symlink; move it away first" >&2; exit 1; \
	fi
	ln -sfnT $(CURDIR)/$* $(PLUGIN_DIR)/$(AUTHOR).$*
	omarchy-shell shell rescanPlugins
	@for i in 1 2 3 4 5; do \
		omarchy plugin enable $(AUTHOR).$* $(PLACE_$*) && exit 0; sleep 1; \
	done; exit 1

$(addprefix uninstall-,$(PLUGINS)): uninstall-%:
	-omarchy plugin disable $(AUTHOR).$*
	rm -f $(PLUGIN_DIR)/$(AUTHOR).$*
	omarchy-shell shell rescanPlugins

install-omafeed:
	$(MAKE) -C omafeed install

uninstall-omafeed:
	$(MAKE) -C omafeed uninstall
