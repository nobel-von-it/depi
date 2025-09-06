CARGO_BIN ?= $(HOME)/.cargo/bin

all: build

build:
	cargo build --release

test: 
	cargo test

testo:
	cargo test -- --show-output

install: test build
	install -D -m 755 target/release/depi $(CARGO_BIN)/depi

clean:
	rm -rf target

uninstall: 
	rm -f $(CARGO_BIN)/depi

.PHONY: build test testo install uninstall clean
