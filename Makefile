build:
	cargo build

format:
	cargo fmt

fmt-check:
	cargo fmt -- --check

lint:
	cargo clippy -- -D warnings

test:
	cargo test

check: fmt-check lint test

.PHONY: format fmt-check lint check test build docker
