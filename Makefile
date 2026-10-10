build:
	cargo build

format:
	cargo fmt

lint:
	cargo clippy -- -D warnings

test:
	cargo test

check: lint test

.PHONY: format lint check test build docker
