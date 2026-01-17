#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "Building release binary..."
cargo build --release

echo "Installing binary to ~/.local/bin..."
mkdir -p ~/.local/bin
cp target/release/btsms ~/.local/bin/

echo "Installing icons..."
mkdir -p ~/.local/share/icons/hicolor/scalable/apps
cp assets/btsms.svg ~/.local/share/icons/hicolor/scalable/apps/

for size in 16 24 32 48 64 128 256; do
    mkdir -p ~/.local/share/icons/hicolor/${size}x${size}/apps
    cp assets/btsms-${size}.png ~/.local/share/icons/hicolor/${size}x${size}/apps/btsms.png
done

echo "Installing desktop entry..."
mkdir -p ~/.local/share/applications
cp assets/btsms.desktop ~/.local/share/applications/

echo "Updating icon cache..."
gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor 2>/dev/null || true

echo "Done! BT SMS is now available in your application menu."
echo "Make sure ~/.local/bin is in your PATH."
