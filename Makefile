CC ?= cc
CFLAGS ?= -O2 -Wall -Wextra -std=c11
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin

all: sbbash

sbbash: sbbash.c
	$(CC) $(CFLAGS) -o $@ $<

install: sbbash
	install -d $(DESTDIR)$(BINDIR)
	install -m 0755 sbbash $(DESTDIR)$(BINDIR)/sbbash

install-perl: sbbash.pl
	install -d $(DESTDIR)$(BINDIR)
	install -m 0755 sbbash.pl $(DESTDIR)$(BINDIR)/sbbash.pl

clean:
	rm -f sbbash *.o

.PHONY: all install install-perl clean
