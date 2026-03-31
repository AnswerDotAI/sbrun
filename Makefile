CC ?= cc
CPPFLAGS ?=
CFLAGS ?= -O2 -Wall -Wextra -std=c11
LDFLAGS ?=
LDLIBS ?= -lsandbox
MACOSX_DEPLOYMENT_TARGET ?= 13.0
export MACOSX_DEPLOYMENT_TARGET
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
XDGCONFIGDIR ?= $(PREFIX)/etc/xdg
CPPFLAGS += -DSBBASH_DEFAULT_XDG_CONFIG_DIRS='"$(XDGCONFIGDIR):/opt/homebrew/etc/xdg:/usr/local/etc/xdg:/etc/xdg"'

all: sbbash

sbbash: sbbash.c
	$(CC) $(CPPFLAGS) $(CFLAGS) $(LDFLAGS) -o $@ $< $(LDLIBS)

install-config: sbbash.default.conf
	install -d $(DESTDIR)$(XDGCONFIGDIR)/sbbash
	[ -f $(DESTDIR)$(XDGCONFIGDIR)/sbbash/config ] || install -m 0644 sbbash.default.conf $(DESTDIR)$(XDGCONFIGDIR)/sbbash/config

install: sbbash install-config
	install -d $(DESTDIR)$(BINDIR)
	install -m 0755 sbbash $(DESTDIR)$(BINDIR)/sbbash

install-perl: sbbash.pl install-config
	install -d $(DESTDIR)$(BINDIR)
	install -m 0755 sbbash.pl $(DESTDIR)$(BINDIR)/sbbash.pl

clean:
	rm -f sbbash *.o

.PHONY: all install install-config install-perl clean
