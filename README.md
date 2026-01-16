# Bluetooth SMS - Cross-Platform SMS Manager

A Linux application for sending/receiving SMS messages via Bluetooth from iPhone and Android devices.

## Features

- ✅ **iPhone Support** - Receive SMS notifications via ANCS, send via MAP
- ✅ **Android Support** - Full SMS send/receive via MAP (Message Access Profile)
- ✅ **Contact Management** - Sync contacts from phone via PBAP
- ✅ **Modern GUI** - GTK4/Libadwaita native interface
- ✅ **Message History** - SQLite database for persistence
- ✅ **Phone Number Normalization** - E.164 format with full test coverage

## Quick Start

### CRITICAL: Start obexd service first!

The app requires the `obexd` service (part of BlueZ) to be running for MAP/PBAP:

```bash
# Start obexd service
systemctl --user start obex

# Or if that doesn't work:
/usr/lib/bluetooth/obexd &
```

### Build and Run

```bash
# Build
cargo build --release

# Run
cargo run --release
```

## Testing

```bash
# Run all tests (30+ tests with full coverage)
cargo test

# Run specific module tests
cargo test phone_normalizer
cargo test vmessage
cargo test database
```

## Architecture

- **Phone Number Normalization**: E.164 format (src/contacts/phone_normalizer.rs)
- **vMessage/BMSG Format**: SMS encoding for MAP (src/bluetooth/vmessage.rs)
- **Database**: SQLite with migrations (src/db/mod.rs)
- **GUI**: GTK4 + Libadwaita (src/gui/mod.rs)

## Requirements

### System Dependencies
```bash
# Debian/Ubuntu
sudo apt-get install libgtk-4-dev libadwaita-1-dev bluez

# Fedora
sudo dnf install gtk4-devel libadwaita-devel bluez

# Arch
sudo pacman -S gtk4 libadwaita bluez
```

### iPhone Setup
1. Pair iPhone with Linux via system Bluetooth
2. On iPhone: `Settings → Bluetooth → [PC] → (i) → Toggle "Show Notifications" ON`
3. Launch application

### Android Setup
1. Pair Android using `bluetoothctl`
2. Grant "Allow message access" permission on device
3. Launch application

## Testing Coverage

All core modules have comprehensive unit tests:
- ✅ Phone normalization (various formats, edge cases)
- ✅ vMessage create/parse (line endings, UTF-8, validation)
- ✅ Database operations (insert, retrieve, migrations)

See [AGENTS.md](AGENTS.md) for testing guidelines.

## License

MIT
