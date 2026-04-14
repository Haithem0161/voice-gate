#!/usr/bin/env bash
#
# Linux virtual-mic setup for VoiceGate. Creates the voicegate_sink and
# voicegate_mic PipeWire nodes and links them. Mirrors PRD Appendix C.
#
# Normally you do NOT need to run this manually -- VoiceGate's own
# PwCliVirtualMic::setup() shells out to pw-cli with the same commands.
# This script exists for:
#   1. The Phase 1 smoke test, so you can verify PipeWire works on your
#      system before running the binary.
#   2. Manual recovery if VoiceGate is killed with SIGKILL and the nodes
#      survive the process (Ctrl-C graceful shutdown handles this for you).
#
# Usage:
#   ./scripts/setup_pipewire.sh           # create the nodes and link them
#   ./scripts/setup_pipewire.sh teardown  # destroy the nodes
#   ./scripts/setup_pipewire.sh verify    # list voicegate nodes
#
set -euo pipefail

SINK_NAME="voicegate_sink"
MIC_NAME="voicegate_mic"

require_pw_cli() {
    if ! command -v pw-cli >/dev/null 2>&1; then
        echo "error: pw-cli not found on PATH. VoiceGate requires PipeWire on Linux." >&2
        echo "       Install with: sudo apt install pipewire pipewire-audio-client-libraries" >&2
        exit 1
    fi
}

cmd_setup() {
    require_pw_cli

    echo "Creating ${SINK_NAME}..."
    pw-cli create-node adapter '{
        factory.name=support.null-audio-sink
        node.name='"${SINK_NAME}"'
        node.description="VoiceGate Sink"
        media.class=Audio/Sink
        audio.position=MONO
        audio.rate=48000
    }'

    echo "Creating ${MIC_NAME}..."
    pw-cli create-node adapter '{
        factory.name=support.null-audio-sink
        node.name='"${MIC_NAME}"'
        node.description="VoiceGate Virtual Microphone"
        media.class=Audio/Source/Virtual
        audio.position=MONO
        audio.rate=48000
    }'

    echo "Linking ${SINK_NAME}:monitor_MONO -> ${MIC_NAME}:input_MONO..."
    pw-link "${SINK_NAME}:monitor_MONO" "${MIC_NAME}:input_MONO"

    echo "Done. Verify with: $0 verify"
    echo "Discord: Settings -> Voice & Video -> Input Device -> VoiceGate Virtual Microphone"
}

cmd_teardown() {
    require_pw_cli
    # Idempotent: errors are suppressed because the nodes may already be gone.
    pw-cli destroy-node "${SINK_NAME}" 2>/dev/null || true
    pw-cli destroy-node "${MIC_NAME}" 2>/dev/null || true
    echo "Teardown complete."
}

cmd_verify() {
    require_pw_cli
    echo "VoiceGate PipeWire nodes:"
    if pw-cli list-objects | grep -E "voicegate_sink|voicegate_mic"; then
        exit 0
    else
        echo "  (none found)"
        exit 1
    fi
}

case "${1:-setup}" in
    setup)    cmd_setup ;;
    teardown) cmd_teardown ;;
    verify)   cmd_verify ;;
    *)
        echo "usage: $0 [setup|teardown|verify]" >&2
        exit 2
        ;;
esac
