.PHONY: help build test test-unit test-integration test-cargo playground clean install fmt clippy check

# Default target
help:
	@echo "Diamond CLI - Development Tasks"
	@echo ""
	@echo "Available targets:"
	@echo "  make build          - Build debug binary"
	@echo "  make release        - Build release binary"
	@echo "  make test           - Run all tests in parallel (nextest)"
	@echo "  make test-unit      - Run only unit tests (nextest)"
	@echo "  make test-integration - Run only integration tests (nextest)"
	@echo "  make test-cargo     - Run tests with cargo test (fallback)"
	@echo "  make playground     - Create isolated test repo for manual testing"
	@echo "  make fmt            - Format code with rustfmt"
	@echo "  make clippy         - Run clippy linter"
	@echo "  make check          - Run fmt + clippy + tests"
	@echo "  make install        - Install dm binary to ~/.cargo/bin"
	@echo "  make clean          - Clean build artifacts and sandbox"

# Build targets
build:
	cargo build

release:
	cargo build --release

# Test targets (using nextest for parallel execution)
test:
	cargo nextest run

test-unit:
	cargo nextest run --bin dm

test-integration:
	cargo nextest run --test '*'

# Fallback for environments without nextest installed
test-cargo:
	cargo test

# Code quality
fmt:
	cargo fmt

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

check: fmt clippy test

# Install
install:
	cargo install --path .

# Clean
clean:
	cargo clean
	rm -rf sandbox/
	rm -rf target/nextest/

# Playground for manual testing
playground:
	@echo "Creating playground test repository..."
	@mkdir -p sandbox
	@cd sandbox && \
		rm -rf test-repo && \
		mkdir test-repo && \
		cd test-repo && \
		git init && \
		git config user.name "Test User" && \
		git config user.email "test@example.com" && \
		echo "# Test Repository" > README.md && \
		git add . && \
		git commit -m "Initial commit" && \
		../../target/debug/dm init
	@echo ""
	@echo "Playground created at: sandbox/test-repo"
	@echo ""
	@echo "To use:"
	@echo "  cd sandbox/test-repo"
	@echo "  ../../target/debug/dm <command>"
	@echo ""
	@echo "Or add an alias:"
	@echo "  alias dm='../../target/debug/dm'"
	@echo ""
