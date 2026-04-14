.PHONY: setup models dev devices run-passthrough test lint fmt fmt-check clippy build release package-linux package-windows clean help

# Default target
help:
	@echo "VoiceGate -- make targets:"
	@echo ""
	@echo "  setup            Install rust components, cargo-watch, cargo-bundle, cargo-wix"
	@echo "  models           Download/export ONNX models into ./models/"
	@echo "  dev              cargo watch -x run (dev loop)"
	@echo "  devices          Run 'voicegate devices' to list cpal devices"
	@echo "  run-passthrough  Run 'voicegate run --passthrough' (Phase 1 smoke test)"
	@echo "  test             cargo test (unit + integration)"
	@echo "  lint             cargo fmt --check + cargo clippy -- -D warnings"
	@echo "  fmt              cargo fmt"
	@echo "  fmt-check        cargo fmt --check"
	@echo "  clippy           cargo clippy -- -D warnings"
	@echo "  build            cargo build (debug)"
	@echo "  release          cargo build --release"
	@echo "  package-linux    Build Linux AppImage/tarball (Phase 6)"
	@echo "  package-windows  Build Windows MSI via cargo-wix (Phase 6)"
	@echo "  clean            cargo clean"

# Install dev tooling
setup:
	rustup component add rustfmt clippy
	cargo install cargo-watch
	cargo install cargo-bundle || true
	cargo install cargo-wix || true
	@echo "Setup complete. Run 'make models' next to fetch ONNX models."

# Download and export ML models
models:
	python3 scripts/download_models.py
	python3 scripts/export_ecapa.py
	@echo "Models written to ./models/"

# Development loop (requires cargo-watch)
dev:
	cargo watch -x 'run -- run --passthrough'

# Run the devices subcommand
devices:
	cargo run --release -- devices

# Run the Phase 1 audio passthrough smoke test
run-passthrough:
	cargo run --release -- run --passthrough

# All tests (unit + integration)
test:
	cargo test

# Full lint gate (matches CI)
lint: fmt-check clippy

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy --all-targets -- -D warnings

build:
	cargo build

release:
	cargo build --release

# Phase 6 packaging targets (placeholders until Phase 6)
package-linux:
	@echo "Phase 6: AppImage / tarball packaging not yet implemented"
	@exit 1

package-windows:
	@echo "Phase 6: cargo-wix MSI packaging not yet implemented"
	@exit 1

clean:
	cargo clean
