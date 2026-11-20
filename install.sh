#!/bin/bash
# Build and install BT SMS for the current user (~/.local).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

PREFIX="${PREFIX:-$HOME/.local}"
APP_ID="com.github.btsms"

echo "Building release binaries..."
cargo build --release --locked

echo "Installing binaries to $PREFIX/bin..."
install -Dm755 target/release/btsms "$PREFIX/bin/btsms"
install -Dm755 target/release/btsms-cli "$PREFIX/bin/btsms-cli"

echo "Installing icons..."
install -Dm644 assets/btsms.svg "$PREFIX/share/icons/hicolor/scalable/apps/btsms.svg"
for size in 16 24 32 48 64 128 256; do
    install -Dm644 "assets/btsms-${size}.png" \
        "$PREFIX/share/icons/hicolor/${size}x${size}/apps/btsms.png"
done

echo "Installing desktop entry..."
# Older versions installed it under a name GNOME could not match to the window.
rm -f "$PREFIX/share/applications/btsms.desktop"
install -Dm644 "assets/${APP_ID}.desktop" "$PREFIX/share/applications/${APP_ID}.desktop"

gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true

echo "Done. BT SMS is in your application menu; make sure $PREFIX/bin is in PATH."
