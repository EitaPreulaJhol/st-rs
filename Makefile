# st - simple terminal (Rust port)
# See LICENSE file for copyright and license details.
.POSIX:

PREFIX = /usr/local
MANPREFIX = $(PREFIX)/share/man
DESTDIR =
VERSION = 0.9.3
CARGO = cargo
DOCKER_COMPOSE = docker compose
RELEASE = target/release/st

all: st

st: Cargo.toml Cargo.lock $(wildcard src/*.rs)
	$(CARGO) build --release
	cp -f $(RELEASE) st

# The full suite (unit tests and X11 end-to-end tests) runs headless in
# Docker. See tests/docker/run.sh.
test: docker-test

docker-test:
	$(DOCKER_COMPOSE) build test
	$(DOCKER_COMPOSE) run --rm test

# Only the Rust unit tests, on the host (no X server, no Docker).
test-local:
	$(CARGO) test --release --locked

# Remove the cached Cargo target volume used by the Docker test service.
test-clean:
	$(DOCKER_COMPOSE) down -v --remove-orphans

clean:
	$(CARGO) clean
	rm -f st st-$(VERSION).tar.gz

dist: clean
	mkdir -p st-$(VERSION)
	cp -R Cargo.toml Cargo.lock README.md LICENSE Makefile FAQ LEGACY TODO \
		config.def.rs st.info st.1 src docker tests \
		compose.yaml .dockerignore st-$(VERSION)
	tar -cf - st-$(VERSION) | gzip > st-$(VERSION).tar.gz
	rm -rf st-$(VERSION)

install: st
	mkdir -p $(DESTDIR)$(PREFIX)/bin
	cp -f st $(DESTDIR)$(PREFIX)/bin
	chmod 755 $(DESTDIR)$(PREFIX)/bin/st
	mkdir -p $(DESTDIR)$(MANPREFIX)/man1
	sed "s/VERSION/$(VERSION)/g" < st.1 > $(DESTDIR)$(MANPREFIX)/man1/st.1
	chmod 644 $(DESTDIR)$(MANPREFIX)/man1/st.1
	tic -sx st.info
	@echo Please see the README file regarding the terminfo entry of st.

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/st
	rm -f $(DESTDIR)$(MANPREFIX)/man1/st.1

.PHONY: all test clean dist install uninstall
