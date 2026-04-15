#!/usr/bin/env bash
set -euo pipefail

# Build an AppImage for VoiceGate.
# Prerequisites: cargo build --release, models downloaded, appimagetool on PATH or at /tmp/appimagetool

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
APPDIR="$REPO_ROOT/target/AppDir"
APPIMAGETOOL="${APPIMAGETOOL:-$(which appimagetool 2>/dev/null || echo /tmp/appimagetool)}"

if [ ! -x "$APPIMAGETOOL" ]; then
    echo "appimagetool not found. Download it:"
    echo "  wget https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage -O /tmp/appimagetool"
    echo "  chmod +x /tmp/appimagetool"
    exit 1
fi

BINARY="$REPO_ROOT/target/release/voicegate"
if [ ! -f "$BINARY" ]; then
    echo "Release binary not found. Run: cargo build --release"
    exit 1
fi

echo "Assembling AppDir..."
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/lib"
mkdir -p "$APPDIR/usr/share/models"
mkdir -p "$APPDIR/usr/share/assets"
mkdir -p "$APPDIR/usr/share/icons/hicolor/64x64/apps"
mkdir -p "$APPDIR/usr/share/applications"

# Binary
cp "$BINARY" "$APPDIR/usr/bin/voicegate"

# Models
if [ -f "$REPO_ROOT/models/silero_vad.onnx" ]; then
    cp "$REPO_ROOT/models/silero_vad.onnx" "$APPDIR/usr/share/models/"
fi
if [ -f "$REPO_ROOT/models/wespeaker_resnet34_lm.onnx" ]; then
    cp "$REPO_ROOT/models/wespeaker_resnet34_lm.onnx" "$APPDIR/usr/share/models/"
fi

# Assets
if [ -d "$REPO_ROOT/assets" ]; then
    cp -r "$REPO_ROOT/assets/"* "$APPDIR/usr/share/assets/" 2>/dev/null || true
fi

# ONNX Runtime shared library
if [ -f /usr/local/lib/libonnxruntime.so ]; then
    cp /usr/local/lib/libonnxruntime.so* "$APPDIR/usr/lib/" 2>/dev/null || true
fi

# Desktop entry and icon
cp "$REPO_ROOT/packaging/linux/voicegate.desktop" "$APPDIR/"
cp "$REPO_ROOT/packaging/linux/voicegate.desktop" "$APPDIR/usr/share/applications/"
cp "$REPO_ROOT/packaging/linux/voicegate.png" "$APPDIR/voicegate.png"
cp "$REPO_ROOT/packaging/linux/voicegate.png" "$APPDIR/usr/share/icons/hicolor/64x64/apps/voicegate.png"
cp "$REPO_ROOT/packaging/linux/voicegate.png" "$APPDIR/.DirIcon"

# AppRun script
cat > "$APPDIR/AppRun" << 'APPRUN_EOF'
#!/bin/bash
SELF="$(readlink -f "$0")"
APPDIR="$(dirname "$SELF")"
export LD_LIBRARY_PATH="$APPDIR/usr/lib:${LD_LIBRARY_PATH:-}"
export VOICEGATE_MODELS_DIR="$APPDIR/usr/share/models"
export VOICEGATE_ASSETS_DIR="$APPDIR/usr/share/assets"
exec "$APPDIR/usr/bin/voicegate" "$@"
APPRUN_EOF
chmod +x "$APPDIR/AppRun"

echo "Building AppImage..."
ARCH=x86_64 "$APPIMAGETOOL" "$APPDIR" "$REPO_ROOT/target/voicegate-x86_64.AppImage"

echo ""
echo "AppImage built: $REPO_ROOT/target/voicegate-x86_64.AppImage"
echo "Size: $(du -h "$REPO_ROOT/target/voicegate-x86_64.AppImage" | cut -f1)"
