.PHONY: check fmt clippy test build release setup

check: fmt-check clippy test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test

build:
	cargo build

release:
	cargo build --release

setup:
	git config core.hooksPath .githooks
	@echo "Git hooks configured!"
