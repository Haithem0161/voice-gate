#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
VERSION="0.1.0"
ARCH="amd64"
PKG_NAME="voicegate_${VERSION}_${ARCH}"
PKG_DIR="$REPO_ROOT/target/$PKG_NAME"

BINARY="$REPO_ROOT/target/release/voicegate"
if [ ! -f "$BINARY" ]; then
    echo "Release binary not found. Run: cargo build --release"
    exit 1
fi

echo "Assembling .deb package..."
rm -rf "$PKG_DIR"

# Directory structure
mkdir -p "$PKG_DIR/DEBIAN"
mkdir -p "$PKG_DIR/usr/bin"
mkdir -p "$PKG_DIR/usr/share/voicegate/models"
mkdir -p "$PKG_DIR/usr/share/voicegate/assets"
mkdir -p "$PKG_DIR/usr/share/applications"
mkdir -p "$PKG_DIR/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$PKG_DIR/usr/lib"

# Binary
cp "$BINARY" "$PKG_DIR/usr/bin/voicegate"
strip "$PKG_DIR/usr/bin/voicegate" 2>/dev/null || true

# Models
[ -f "$REPO_ROOT/models/silero_vad.onnx" ] && cp "$REPO_ROOT/models/silero_vad.onnx" "$PKG_DIR/usr/share/voicegate/models/"
[ -f "$REPO_ROOT/models/wespeaker_resnet34_lm.onnx" ] && cp "$REPO_ROOT/models/wespeaker_resnet34_lm.onnx" "$PKG_DIR/usr/share/voicegate/models/"

# Assets
cp -r "$REPO_ROOT/assets/"* "$PKG_DIR/usr/share/voicegate/assets/" 2>/dev/null || true

# ONNX Runtime
if [ -f /usr/local/lib/libonnxruntime.so ]; then
    cp /usr/local/lib/libonnxruntime.so* "$PKG_DIR/usr/lib/" 2>/dev/null || true
fi

# Desktop entry
cat > "$PKG_DIR/usr/share/applications/voicegate.desktop" << 'DESKTOP'
[Desktop Entry]
Name=VoiceGate
Comment=Real-time speaker isolation for Discord
Exec=voicegate run
Icon=voicegate
Terminal=false
Type=Application
Categories=Audio;AudioVideo;
DESKTOP

# Icon
cp "$REPO_ROOT/packaging/linux/voicegate.png" "$PKG_DIR/usr/share/icons/hicolor/256x256/apps/voicegate.png"

# Calculate installed size in KB
INSTALLED_SIZE=$(du -sk "$PKG_DIR" | cut -f1)

# Control file
cat > "$PKG_DIR/DEBIAN/control" << EOF
Package: voicegate
Version: $VERSION
Section: sound
Priority: optional
Architecture: $ARCH
Depends: libasound2, libpipewire-0.3-0 | pulseaudio-utils
Installed-Size: $INSTALLED_SIZE
Maintainer: Haithem <cloud.torchcorp@gmail.com>
Description: Real-time speaker isolation for Discord
 VoiceGate gates your microphone using neural speaker verification
 (Silero VAD + WeSpeaker) and routes clean audio to a virtual
 microphone for Discord.
EOF

# Post-install: update icon cache + ldconfig
cat > "$PKG_DIR/DEBIAN/postinst" << 'POSTINST'
#!/bin/sh
ldconfig
gtk-update-icon-cache /usr/share/icons/hicolor 2>/dev/null || true
POSTINST
chmod 755 "$PKG_DIR/DEBIAN/postinst"

# Build the .deb
dpkg-deb --build "$PKG_DIR"

echo ""
echo ".deb built: $REPO_ROOT/target/${PKG_NAME}.deb"
echo "Size: $(du -h "$REPO_ROOT/target/${PKG_NAME}.deb" | cut -f1)"
echo ""
echo "Install with: sudo dpkg -i target/${PKG_NAME}.deb"
