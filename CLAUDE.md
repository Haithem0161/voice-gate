# VoiceGate

**Real-time speaker isolation for Discord.** Cross-platform Rust desktop binary that gates the user's microphone using neural speaker verification (Silero VAD + ECAPA-TDNN) and routes clean audio to a virtual microphone (VB-Cable on Windows, PipeWire virtual source on Linux).

Tech Stack: Rust 1.83 | cpal (audio I/O) | ort + ONNX Runtime (ML) | rubato (resampling) | eframe/egui (GUI) | ringbuf (lock-free SPSC) | hound (WAV)

**Targets:** `x86_64-unknown-linux-gnu` (Ubuntu 22.04+), `x86_64-pc-windows-msvc` (Windows 10/11). macOS is out of scope for v1.

## Core Principles

1. **Real-Time Safety First**: Audio callbacks (cpal input/output) are owned by the OS and run at real-time priority. NEVER allocate, lock a mutex, call into the kernel, log, or do anything that can block inside a callback. Callbacks only push/pop from a `ringbuf` SPSC queue. All real work happens on a separate processing thread.
2. **Module Boundaries**: `audio/` depends only on cpal/ringbuf/rubato. `ml/` depends only on ort/ndarray. `enrollment/`, `gate/`, `similarity` are pure logic over `&[f32]`. `gui/` never imports cpal or ort directly -- it talks to a single `AppController`. No upward imports.
3. **Context7 First**: NEVER write code using a crate without first querying Context7 for its current API. Your training data is older than the crate's latest release. This is mandatory, not optional.
4. **No Emojis**: Never use emojis in code, comments, docs, commit messages, or user-facing text. This is a hard rule.
5. **Makefile Orchestration**: Use `make` commands for all workflow tasks. Never run raw commands when a make target exists.
6. **Frame Size is Locked**: 32 ms frames end-to-end (1536 samples at 48 kHz, 512 samples at 16 kHz). This aligns with Silero VAD's required input size. Any other value is rejected at config load time. See Decision D-001 in `docs/voicegate/research.md`.

## Critical Rules

### Git Commits
**NEVER commit with Claude authorship or co-authorship.** No `Co-Authored-By: Claude`, no Claude/Anthropic emails, no "Generated with Claude Code" trailer, no modifying git config. All commits must appear as solely human-made. No emojis in commit messages.

### Context7 Documentation Lookup (MANDATORY)
Before writing ANY implementation code using a crate:
1. Call `resolve-library-id` to find the library
2. Call `query-docs` with your specific use case
3. Use the returned docs/examples as the basis for implementation

Applies to: `cpal`, `ort`, `eframe`, `egui`, `rubato`, `hound`, `ringbuf`, `ndarray`, `dirs`, `thiserror`, `serde`, `toml`, `anyhow`, `tracing`, `clap`, `which`, `pipewire` (Linux only). The API shape of every one of these has shifted in the last release cycle. Do not rely on memory.

### Crate Management
- NEVER manually edit `Cargo.toml` dependency versions. Use `cargo add <crate>`.
- Exception: the initial `Cargo.toml` from Phase 1 is hand-written to pin all versions in one place; subsequent changes go through `cargo add`.

### Destructive Operations
- NEVER run `rm -rf` or `git clean -fd` without confirming with the user first.
- NEVER force-push.
- NEVER skip git hooks (`--no-verify`). `.claude/hooks/block-force-push.sh` and `block-docker-destroy.sh` exist for a reason.

## Development Workflow

1. **Research (MANDATORY)** -- Query Context7 for every crate before writing code. Study existing patterns in `src/`.
2. **Plan** -- If the work is non-trivial, check `docs/voicegate/phase-XX.md` for the relevant phase specification. Follow it.
3. **Implement** -- Keep modules small. Respect boundaries (see Principle 2).
4. **Verify** -- `make lint` (`cargo fmt --check` + `cargo clippy -- -D warnings`) before every commit. `make test` for automated tests. Manual smoke tests with fixture WAVs for audio paths.
5. **Commit** -- Small, atomic, with a message that explains WHY not what. No Claude attribution. No emojis.

## File Paths (not ports)

VoiceGate is a desktop app. There are no ports. The runtime state is on disk.

| Path | Purpose |
|------|---------|
| `dirs::config_dir()/voicegate/config.toml` | User-editable TOML config (PRD §5.9) |
| `dirs::data_dir()/voicegate/profile.bin` | Enrolled speaker profile (`VGPR` magic + CRC32) |
| `models/silero_vad.onnx` | Silero VAD model (~2 MB, downloaded) |
| `models/ecapa_tdnn.onnx` | ECAPA-TDNN embedding model (~80 MB, exported) |
| `tests/fixtures/*.wav` | Test WAVs for ML discrimination and pipeline tests |
| `assets/enrollment_passages.txt` | PRD Appendix A pangram passage |
| `scripts/setup_pipewire.sh` | Linux virtual-mic setup via `pw-cli` (Phase 1 path) |
| `scripts/download_models.py` | Fetch Silero VAD ONNX |
| `scripts/export_ecapa.py` | Export SpeechBrain ECAPA-TDNN to ONNX with PyTorch/ONNX equivalence check |

## Testing (Fixture WAVs, not curl)

```bash
# Run all tests (unit + integration)
make test

# Run a specific integration test
cargo test --test test_ml test_embedding_discrimination

# Manual audio passthrough smoke test (Linux, Phase 1)
./scripts/setup_pipewire.sh
cargo run --release -- run --passthrough
# In another terminal, record from voicegate_mic and verify the audio matches

# List cpal devices
cargo run --release -- devices
```

No database setup. No curl commands. No Swagger UI. Audio paths are verified with deterministic WAV fixtures in `tests/fixtures/`, or with a loopback smoke test using `pw-cat --record` / Audacity.

## Common Pitfalls

- **Allocating in an audio callback**: the #1 way to get audio dropouts and xruns. Callbacks must only `push_slice` / `pop_slice` on the ring buffer. No `Vec::new()`, no `println!`, no `Mutex::lock()`, no `tracing::info!`. Pre-allocate everything, or use stack arrays.
- **Frame size mismatch**: Silero VAD expects exactly 512 samples at 16 kHz. At 48 kHz input that's 1536 samples per frame. Using 20 ms (960 samples) or 30 ms frames silently breaks VAD. `Config::validate()` rejects non-32 values at load time.
- **Silero VAD GRU state**: the model is stateful. The 2×1×128 hidden state MUST persist across calls. Do NOT recreate the ORT session or reset the state array per frame. Re-initialize state only on silence gaps longer than ~500 ms.
- **ECAPA-TDNN embedding not L2-normalized**: cosine similarity on non-normalized vectors is not cosine similarity. Both enrollment centroid and live embedding must be L2-normalized before `cosine_similarity`.
- **cpal `BufferSize::Fixed(1536)` rejected**: some ALSA devices don't support fixed-size buffers. The capture path must fall back to `BufferSize::Default` and let the ring buffer + worker thread re-chunk into 1536-sample frames. Do not assume the callback size.
- **Gate crossfade in ms vs samples**: `GateConfig` stores `crossfade_ms: f32`. The gate itself works in samples. Convert once via `GateConfig::crossfade_samples(sample_rate)`. Hard-coding 256 is wrong -- 5 ms at 48 kHz is exactly 240 samples.
- **ECAPA-TDNN ONNX export wrong submodule**: `torch.onnx.export` on a SpeechBrain model can produce an ONNX that runs but returns garbage if the wrong subgraph is traced. The export script MUST include a PyTorch/ONNX equivalence check asserting `max(|pt - ort| < 1e-4)`.
- **PipeWire nodes leak on SIGKILL**: if the process is killed hard, `voicegate_sink` and `voicegate_mic` survive. Recovery is `pw-cli destroy-node voicegate_sink && pw-cli destroy-node voicegate_mic`. Graceful Ctrl-C handles this via the `VirtualMic::teardown()` path.
- **Atomic f32 does not exist in stable Rust**: the PRD mentions `Arc<AtomicF32>` but the actual implementation is `Arc<AtomicU32>` with `f32::to_bits` / `f32::from_bits`. See `src/pipeline/` in Phase 4.
- **`thiserror` v2 `#[error(...)]` format strings**: use `{0}` for tuple variants, `{field_name}` for named fields. `#[from]` auto-generates `From` impls. v2 is stricter than v1 about unused format args.

## Detailed Rules (auto-loaded by path)

Architecture details, patterns, and conventions live in `.claude/rules/`:

- `rust-desktop.md` -- Binary crate layout, module tree, `AppState`, `thiserror` boundaries, avoiding async where not needed
- `audio-io.md` -- cpal device selection, frame alignment, ring buffer patterns, rubato resampling, hound WAV I/O, ALSA/WASAPI quirks
- `ml-inference.md` -- ort session creation, execution providers, Silero VAD stateful handling, ECAPA-TDNN tensor shape and L2 normalization, cosine similarity, mandatory Context7 query before touching ort
- `gui.md` -- eframe/egui app lifecycle, `eframe::App::update`, cross-thread repaint, `rfd` file dialogs, no blocking in `update`
- `module-boundaries.md` -- Layer rules, no upward imports, GUI talks only to `AppController`
- `cross-platform.md` -- Target triples, `#[cfg(target_os)]` patterns, `dirs` paths, ONNX Runtime shared-library handling per OS, packaging (cargo-wix for MSI, AppImage for Linux)
- `testing.md` -- Unit tests inline, integration tests in `tests/` reading fixture WAVs via hound, deterministic ORT session tests, embedding-vector snapshot tests with tolerance
- `planning.md` -- Plan writing methodology. VoiceGate uses the **desktop-adapted** 7-section phase template (see the file's header note)

<!-- MEMORY:START -->
# fullstack-rust-react

_Last updated: 2026-04-14 | 0 active memories, 0 total_

_For deeper context, use memory_search, memory_related, or memory_ask tools._
<!-- MEMORY:END -->
