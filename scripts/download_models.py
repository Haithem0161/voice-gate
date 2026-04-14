#!/usr/bin/env python3
# Phase 2 placeholder. Downloads the Silero VAD ONNX model into ./models/.
# Real implementation lands in Phase 2 per docs/voicegate/phase-02.md.
#
# Planned behavior (Phase 2):
#   1. Fetch https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx
#   2. Verify file size is in [1, 5] MB
#   3. Write to ./models/silero_vad.onnx
#   4. Print a one-line success message
#
# For Phase 1 this script exists only so subsequent commits can edit it
# rather than create it, and so `make models` has a target to call.

def main() -> int:
    print("scripts/download_models.py: not yet implemented (Phase 2)")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
