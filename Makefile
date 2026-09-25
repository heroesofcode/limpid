# Installation. `cargo build --release` first, or use `make build`.
#
# The helper goes to libexec rather than bin: it is not a command anyone
# should run, and the polkit policy names that exact path.

PREFIX  ?= /usr
DESTDIR ?=

BINDIR     = $(DESTDIR)$(PREFIX)/bin
HELPERDIR  = $(DESTDIR)$(PREFIX)/lib/limpid
POLICYDIR  = $(DESTDIR)$(PREFIX)/share/polkit-1/actions
DESKTOPDIR = $(DESTDIR)$(PREFIX)/share/applications
ICONDIR    = $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps
LICENSEDIR = $(DESTDIR)$(PREFIX)/share/licenses/limpid

TARGET = target/release

.PHONY: all build install uninstall check clean

all: build

build:
	cargo build --release --workspace

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --all-features -- -D warnings
	cargo test --workspace

install:
	install -Dm755 $(TARGET)/limpid      $(BINDIR)/limpid
	install -Dm755 $(TARGET)/limpid-cli  $(BINDIR)/limpid-cli
	install -Dm755 $(TARGET)/limpid-helper $(HELPERDIR)/limpid-helper
	install -Dm644 data/org.limpid.helper.policy $(POLICYDIR)/org.limpid.helper.policy
	install -Dm644 data/limpid.desktop   $(DESKTOPDIR)/limpid.desktop
	install -Dm644 data/limpid.svg       $(ICONDIR)/limpid.svg
	install -Dm644 LICENSE               $(LICENSEDIR)/LICENSE

uninstall:
	rm -f  $(BINDIR)/limpid $(BINDIR)/limpid-cli
	rm -f  $(HELPERDIR)/limpid-helper
	rmdir  $(HELPERDIR) 2>/dev/null || true
	rm -f  $(POLICYDIR)/org.limpid.helper.policy
	rm -f  $(DESKTOPDIR)/limpid.desktop
	rm -f  $(ICONDIR)/limpid.svg
	rm -rf $(LICENSEDIR)

clean:
	cargo clean
