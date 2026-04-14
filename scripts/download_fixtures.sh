#!/usr/bin/env bash
#
# Populate tests/fixtures/ with the WAV files needed by Phase 2's ML
# integration test suite:
#
#   speaker_a.wav          ~10 s, LibriSpeech speaker 1272, mono 16 kHz s16le
#   speaker_b.wav          ~10 s, LibriSpeech speaker 1462, mono 16 kHz s16le
#   speaker_a_enroll.wav   ~30 s, LibriSpeech speaker 1272 (different clip)
#   silence.wav            5 s, ffmpeg anullsrc, mono 16 kHz s16le
#   noise.wav              5 s, ffmpeg anoisesrc pink, mono 16 kHz s16le
#
# LibriSpeech dev-clean is CC BY 4.0. It is NOT re-distributed with this
# repo; the tarball is downloaded on demand (~330 MB) and only the three
# required FLACs are extracted. The tarball is deleted after use.
#
# Requirements: curl, tar, ffmpeg (with libswresample), pw-cli-free env.
#
# Usage:
#   ./scripts/download_fixtures.sh           # populate missing fixtures
#   ./scripts/download_fixtures.sh --force   # re-create everything
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURES_DIR="$REPO_ROOT/tests/fixtures"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$FIXTURES_DIR"

force=0
if [[ "${1:-}" == "--force" ]]; then
    force=1
fi

# Sanity: ffmpeg must exist.
if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "error: ffmpeg not found on PATH. Install with: sudo apt install ffmpeg" >&2
    exit 1
fi

# If all five fixtures already exist and we're not forcing, skip.
all_present=1
for f in speaker_a speaker_b speaker_a_enroll silence noise; do
    if [[ ! -f "$FIXTURES_DIR/$f.wav" ]]; then
        all_present=0
        break
    fi
done

if [[ $all_present -eq 1 && $force -eq 0 ]]; then
    echo "All fixtures already present in $FIXTURES_DIR (use --force to re-create)"
    ls -la "$FIXTURES_DIR"/*.wav
    exit 0
fi

synth_silence() {
    local out="$1"
    echo "synthesizing: $(basename "$out") (ffmpeg anullsrc, 5 s, mono 16 kHz)"
    ffmpeg -y -v error \
        -f lavfi -i "anullsrc=r=16000:cl=mono" \
        -t 5 -c:a pcm_s16le \
        "$out"
}

synth_noise() {
    local out="$1"
    echo "synthesizing: $(basename "$out") (ffmpeg anoisesrc pink, 5 s, mono 16 kHz)"
    # anoisesrc defaults: amplitude=1.0 is wildly loud; scale to -20 dBFS.
    ffmpeg -y -v error \
        -f lavfi -i "anoisesrc=d=5:c=pink:r=16000:a=0.1" \
        -ac 1 -ar 16000 -c:a pcm_s16le \
        "$out"
}

download_librispeech_subset() {
    # Download dev-clean and extract only the speakers we need. ~330 MB
    # download, trimmed to ~5 MB on disk after extraction, tarball deleted.
    local tarball="$TMP_DIR/librispeech-dev-clean.tar.gz"
    local url="https://www.openslr.org/resources/12/dev-clean.tar.gz"

    if [[ ! -f "$tarball" ]]; then
        echo "downloading: LibriSpeech dev-clean (~330 MB, one-time)"
        echo "  from: $url"
        curl --fail --location --silent --show-error --output "$tarball" "$url"
    fi

    echo "extracting: LibriSpeech speakers 1272 and 1462"
    tar -xzf "$tarball" -C "$TMP_DIR" \
        'LibriSpeech/dev-clean/1272' \
        'LibriSpeech/dev-clean/1462'
    rm -f "$tarball"
}

flac_to_wav() {
    local src="$1"
    local dst="$2"
    local extra_args="${3:-}"
    echo "transcoding: $(basename "$dst") (from $(basename "$src") $extra_args)"
    # shellcheck disable=SC2086
    ffmpeg -y -v error \
        -i "$src" \
        -ac 1 -ar 16000 -c:a pcm_s16le \
        $extra_args \
        "$dst"
}

# --- Generate fixtures ----------------------------------------------------

need_librispeech=0
for target in speaker_a speaker_b speaker_a_enroll; do
    if [[ ! -f "$FIXTURES_DIR/$target.wav" || $force -eq 1 ]]; then
        need_librispeech=1
    fi
done

if [[ $need_librispeech -eq 1 ]]; then
    download_librispeech_subset
fi

# speaker_a: 9.9 s clip from speaker 1272
if [[ ! -f "$FIXTURES_DIR/speaker_a.wav" || $force -eq 1 ]]; then
    flac_to_wav \
        "$TMP_DIR/LibriSpeech/dev-clean/1272/128104/1272-128104-0003.flac" \
        "$FIXTURES_DIR/speaker_a.wav"
fi

# speaker_b: 14.55 s clip from speaker 1462, trimmed to 10 s
if [[ ! -f "$FIXTURES_DIR/speaker_b.wav" || $force -eq 1 ]]; then
    flac_to_wav \
        "$TMP_DIR/LibriSpeech/dev-clean/1462/170138/1462-170138-0000.flac" \
        "$FIXTURES_DIR/speaker_b.wav" \
        "-t 10"
fi

# speaker_a_enroll: 29.4 s clip from speaker 1272 (different utterance)
if [[ ! -f "$FIXTURES_DIR/speaker_a_enroll.wav" || $force -eq 1 ]]; then
    flac_to_wav \
        "$TMP_DIR/LibriSpeech/dev-clean/1272/128104/1272-128104-0004.flac" \
        "$FIXTURES_DIR/speaker_a_enroll.wav"
fi

# silence.wav
if [[ ! -f "$FIXTURES_DIR/silence.wav" || $force -eq 1 ]]; then
    synth_silence "$FIXTURES_DIR/silence.wav"
fi

# noise.wav
if [[ ! -f "$FIXTURES_DIR/noise.wav" || $force -eq 1 ]]; then
    synth_noise "$FIXTURES_DIR/noise.wav"
fi

echo ""
echo "Fixtures ready in $FIXTURES_DIR:"
ls -la "$FIXTURES_DIR"/*.wav
