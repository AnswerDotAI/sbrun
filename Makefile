CC ?= cc
CPPFLAGS ?=
CFLAGS ?= -O2 -Wall -Wextra -std=c11
LDFLAGS ?=
LDLIBS ?= -lsandbox
VERSION := $(shell cat VERSION)
MACOSX_DEPLOYMENT_TARGET ?= 13.0
export MACOSX_DEPLOYMENT_TARGET
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
XDGCONFIGDIR ?= $(PREFIX)/etc/xdg
CPPFLAGS += -DSBBASH_DEFAULT_XDG_CONFIG_DIRS='"$(XDGCONFIGDIR):/opt/homebrew/etc/xdg:/usr/local/etc/xdg:/etc/xdg"'
CPPFLAGS += -DSBRUN_VERSION='"$(VERSION)"'

all: sbrun

sbrun: sbrun.c
	$(CC) $(CPPFLAGS) $(CFLAGS) $(LDFLAGS) -o $@ $< $(LDLIBS)

install-config: sbrun.default.conf
	install -d $(DESTDIR)$(XDGCONFIGDIR)/sbrun
	[ -f $(DESTDIR)$(XDGCONFIGDIR)/sbrun/config ] || install -m 0644 sbrun.default.conf $(DESTDIR)$(XDGCONFIGDIR)/sbrun/config

install: sbrun install-config
	install -d $(DESTDIR)$(BINDIR)
	install -m 0755 sbrun $(DESTDIR)$(BINDIR)/sbrun

install-perl: sbrun.pl install-config
	install -d $(DESTDIR)$(BINDIR)
	install -m 0755 sbrun.pl $(DESTDIR)$(BINDIR)/sbrun.pl

clean:
	rm -f sbrun *.o

.PHONY: all install install-config install-perl clean
