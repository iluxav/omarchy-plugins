# omafeed — build and install
#
#   make build       build the release binary
#   make run         build and start it
#   make install     build, then copy to ~/.local/bin (no sudo needed)
#   make uninstall   remove it again
#   sudo make install PREFIX=/usr/local     system-wide

PREFIX  ?= $(HOME)/.local
BINDIR  ?= $(PREFIX)/bin
NAME    := omafeed
BIN     := target/release/$(NAME)
SOURCES := Cargo.toml Cargo.lock $(wildcard src/*.rs)

.PHONY: build run install uninstall

build: $(BIN)

# Only calls cargo when a source changed, so `sudo make install` after a plain
# `make` just copies and never runs cargo as root.
$(BIN): $(SOURCES)
	cargo build --release
	@touch $(BIN)

run: $(BIN)
	./$(BIN)

install: $(BIN)
	install -Dm755 $(BIN) $(DESTDIR)$(BINDIR)/$(NAME)
	@echo "Installed $(DESTDIR)$(BINDIR)/$(NAME)"
	@case ":$$PATH:" in \
		*":$(BINDIR):"*) echo "Run it from anywhere:  $(NAME)" ;; \
		*) echo "Note: $(BINDIR) is not on your PATH. Add this to your shell profile:"; \
		   echo "  export PATH=\"$(BINDIR):\$$PATH\"" ;; \
	esac

# Removes the program only. Settings in ~/.config/omafeed and the saved feed
# in ~/.local/state/omafeed are left alone.
uninstall:
	rm -f $(DESTDIR)$(BINDIR)/$(NAME)
	@echo "Removed $(DESTDIR)$(BINDIR)/$(NAME)"
