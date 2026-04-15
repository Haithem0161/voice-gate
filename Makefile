.PHONY: setup models fixtures dev devices run-passthrough test lint fmt fmt-check clippy build release package-linux package-windows clean help

# Default target
help:
	@echo "VoiceGate -- make targets:"
	@echo ""
	@echo "  setup            Install rust components, cargo-watch, cargo-bundle, cargo-wix"
	@echo "  models           Download ONNX models into ./models/"
	@echo "  fixtures         Download LibriSpeech clips and synthesize silence/noise into ./tests/fixtures/"
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

# Download ONNX models (silero_vad.onnx + wespeaker_resnet34_lm.onnx)
models:
	bash scripts/download_models.sh
	@echo "Models written to ./models/"

# Download + synthesize test fixtures (Phase 2+)
fixtures:
	bash scripts/download_fixtures.sh
	@echo "Fixtures written to ./tests/fixtures/"

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

# Package as AppImage (Linux). Requires appimagetool.
package-linux: release
	bash scripts/package-appimage.sh

# Package as MSI (Windows). Requires cargo-wix.
package-windows: release
	cargo wix

clean:
	cargo clean
