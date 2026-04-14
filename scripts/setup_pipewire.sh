#!/usr/bin/env bash
#
# Linux virtual-mic quick reference for VoiceGate.
#
# Uses pw-loopback (the long-running helper that is the correct tool on
# modern PipeWire 1.x) to create a voicegate_sink + voicegate_mic pair.
# The voicegate_sink is what VoiceGate's cpal output writes to; the
# voicegate_mic is what Discord reads as an input device.
#
# An earlier version of this script used `pw-cli create-node` +
# `pw-link`, matching PRD Appendix C literally. That approach does not
# work on PipeWire 1.0.x because `pw-cli create-node` creates a node
# owned by the short-lived pw-cli process and destroyed the moment the
# process exits. See phase-01.md section 6.3 for the full discovery.
#
# Normally you do NOT need to run this manually -- VoiceGate's own
# PwCliVirtualMic::setup() spawns pw-loopback with the same arguments.
# This script exists for:
#   1. The Phase 1 smoke test, so you can verify PipeWire works on your
#      system before running the binary.
#   2. Manual recovery if VoiceGate is killed and the loopback process
#      somehow survives (graceful Ctrl-C handles this for you; SIGKILL
#      leaves no loopback to clean up because it dies with its parent).
#
# Usage:
#   ./scripts/setup_pipewire.sh           # create the virtual devices (runs until Ctrl-C)
#   ./scripts/setup_pipewire.sh verify    # list voicegate nodes in another terminal
#   ./scripts/setup_pipewire.sh teardown  # kill any stray pw-loopback processes
#
set -euo pipefail

SINK_NAME="voicegate_sink"
MIC_NAME="voicegate_mic"

require_pw_loopback() {
    if ! command -v pw-loopback >/dev/null 2>&1; then
        echo "error: pw-loopback not found on PATH. VoiceGate requires PipeWire on Linux." >&2
        echo "       Install with: sudo apt install pipewire pipewire-audio-client-libraries" >&2
        exit 1
    fi
}

cmd_setup() {
    require_pw_loopback
    echo "Starting pw-loopback (Ctrl-C to stop)..."
    echo "  sink: ${SINK_NAME}"
    echo "  mic:  ${MIC_NAME}"
    echo ""
    echo "In another terminal run '$0 verify' to confirm the nodes exist."
    echo "Point Discord at '${MIC_NAME}' as its input device."
    echo ""
    exec pw-loopback \
        --channels 1 \
        --capture-props "node.name=${SINK_NAME} node.description=\"VoiceGate Sink\" media.class=Audio/Sink" \
        --playback-props "node.name=${MIC_NAME} node.description=\"VoiceGate Virtual Microphone\" media.class=Audio/Source/Virtual"
}

cmd_teardown() {
    # Kill any stray pw-loopback processes that are owning our named nodes.
    # pgrep -f matches on the full command line, so we match on the node name.
    local killed=0
    for pid in $(pgrep -f "pw-loopback.*${SINK_NAME}" 2>/dev/null || true); do
        kill "$pid" 2>/dev/null && killed=1
    done
    if [[ $killed -eq 1 ]]; then
        echo "Stray pw-loopback processes killed."
    else
        echo "(nothing to tear down)"
    fi
}

cmd_verify() {
    if ! command -v pw-cli >/dev/null 2>&1; then
        echo "error: pw-cli not found on PATH." >&2
        exit 1
    fi
    echo "VoiceGate PipeWire nodes:"
    if pw-cli ls Node 2>/dev/null | grep -E "node\.name = \"voicegate_"; then
        exit 0
    else
        echo "  (none found -- run '$0 setup' in another terminal first)"
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
