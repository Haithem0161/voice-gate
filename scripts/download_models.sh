#!/usr/bin/env bash
#
# Download the ONNX models VoiceGate needs at runtime.
#
# Models fetched:
#   1. silero_vad.onnx             -- ~2.2 MB from github.com/snakers4/silero-vad
#   2. wespeaker_resnet34_lm.onnx  -- ~25 MB from huggingface.co/Wespeaker
#
# Per Decision D-002R (2026-04-14), VoiceGate uses WeSpeaker's pre-exported
# ResNet34-LM ONNX as the primary speaker embedding model. The "LM" variant
# is large-margin fine-tuned and performs better on >3s audio clips per
# WeSpeaker's documentation, which matches VoiceGate's typical enrollment
# and runtime speaker-verification window.
#
# The script is idempotent: files that already exist and match their
# expected SHA-256 are skipped. Corrupt or MITM'd downloads are caught
# by the checksum verification. If any file is stale or wrong, delete
# models/ and re-run this script.
#
# Usage:
#   ./scripts/download_models.sh           # download missing/bad models
#   ./scripts/download_models.sh --force   # re-download everything
#
set -euo pipefail

MODELS_DIR="$(cd "$(dirname "$0")/.." && pwd)/models"
mkdir -p "$MODELS_DIR"

# Model table: one entry per model. Format:
#   <local_filename>|<url>|<expected_sha256>
MODELS=(
    "silero_vad.onnx|https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx|1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3"
    "wespeaker_resnet34_lm.onnx|https://huggingface.co/Wespeaker/wespeaker-resnet34-LM/resolve/main/voxceleb_resnet34_LM.onnx?download=true|7bb2f06e9df17cdf1ef14ee8a15ab08ed28e8d0ef5054ee135741560df2ec068"
)

# Portable SHA-256: use sha256sum on Linux, shasum -a 256 on macOS.
sha256_of() {
    local path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    else
        echo "error: neither sha256sum nor shasum found on PATH" >&2
        exit 1
    fi
}

force=0
if [[ "${1:-}" == "--force" ]]; then
    force=1
fi

for entry in "${MODELS[@]}"; do
    IFS='|' read -r filename url expected_sha <<< "$entry"
    dest="$MODELS_DIR/$filename"

    if [[ -f "$dest" && $force -eq 0 ]]; then
        actual_sha="$(sha256_of "$dest")"
        if [[ "$actual_sha" == "$expected_sha" ]]; then
            echo "ok: $filename ($(wc -c < "$dest") bytes, sha256 verified)"
            continue
        else
            echo "warn: $filename exists but checksum mismatch; re-downloading"
            rm -f "$dest"
        fi
    fi

    echo "downloading: $filename"
    echo "  from: $url"
    # --fail: non-zero exit on HTTP >= 400
    # --location: follow redirects (HuggingFace and GitHub both use 302s)
    # --silent --show-error: progress off but errors still visible
    # --output: write to file instead of stdout
    if ! curl --fail --location --silent --show-error --output "$dest.tmp" "$url"; then
        echo "error: download failed for $filename" >&2
        rm -f "$dest.tmp"
        exit 1
    fi

    actual_sha="$(sha256_of "$dest.tmp")"
    if [[ "$actual_sha" != "$expected_sha" ]]; then
        echo "error: $filename checksum mismatch" >&2
        echo "  expected: $expected_sha" >&2
        echo "  actual:   $actual_sha" >&2
        rm -f "$dest.tmp"
        exit 1
    fi

    mv "$dest.tmp" "$dest"
    echo "ok: $filename ($(wc -c < "$dest") bytes, sha256 verified)"
done

echo ""
echo "Models ready in $MODELS_DIR"
ls -la "$MODELS_DIR" | grep -E '\.onnx$' || true
