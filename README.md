# Bluetooth SMS - Cross-Platform SMS Manager

A Linux application for sending and receiving SMS messages via Bluetooth from iPhone and Android devices. Think of it as a Linux equivalent to Microsoft's Phone Link.

> **Note**: This project was entirely vibe coded with AI assistance. Use at your own risk.

## Features

- **iPhone Support** - Receive SMS notifications via ANCS, send via MAP
- **Android Support** - Full SMS send/receive via MAP (Message Access Profile)
- **Contact Management** - Sync contacts from phone via PBAP
- **Modern GUI** - GTK4/Libadwaita native interface
- **Message History** - SQLite database for persistence
- **Phone Number Normalization** - E.164 format with full test coverage

## Known Issues

> **Contact Sync is Broken**: As of my experience, contact synchronization via PBAP does not work reliably. Phone numbers will display without contact names. This is a known limitation.

## Protocol Limitations

This application uses standard Bluetooth profiles (MAP, PBAP, ANCS) which have inherent limitations:

- **No Historical Sync**: You can only see messages received while the app is connected. You cannot download the phone's entire message history - Apple and Android restrict this.
- **Text Only**: MAP supports basic text messages only. You cannot send or receive photos, videos, or other media.
- **Group Chats**: Poor support for group threads. Replies are typically treated as individual 1-to-1 messages.
- **Connection Required**: Messages are only received while Bluetooth is actively connected. There's no background sync.
- **iPhone Quirks**:
  - Requires dual-protocol connection (BLE for notifications, Classic BT for sending)
  - User must manually enable "Show Notifications" in Bluetooth settings
  - iMessage vs SMS is decided by the iPhone automatically based on recipient

## Dependencies

### Runtime Dependencies

These packages are required to run the application:

**Debian/Ubuntu:**
```bash
sudo apt install libgtk-4-1 libadwaita-1-0 bluez sqlite3
```

**Fedora:**
```bash
sudo dnf install gtk4 libadwaita bluez sqlite
```

**Arch Linux:**
```bash
sudo pacman -S gtk4 libadwaita bluez sqlite
```

**openSUSE:**
```bash
sudo zypper install libgtk-4-1 libadwaita-1-0 bluez sqlite3
```

### Build Dependencies

These packages are required to compile the application from source (includes runtime dependencies):

**Debian/Ubuntu:**
```bash
sudo apt install libgtk-4-dev libadwaita-1-dev bluez sqlite3 libsqlite3-dev pkg-config build-essential
```

**Fedora:**
```bash
sudo dnf install gtk4-devel libadwaita-devel bluez sqlite-devel pkg-config gcc
```

**Arch Linux:**
```bash
sudo pacman -S gtk4 libadwaita bluez sqlite pkg-config base-devel
```

**openSUSE:**
```bash
sudo zypper install gtk4-devel libadwaita-devel bluez sqlite3-devel pkg-config gcc
```

You also need the Rust toolchain. Install it via [rustup](https://rustup.rs/):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Installation

### Using the Install Script (Recommended)

```bash
git clone https://github.com/user/btsms.git
cd btsms
./install.sh
```

This will:
- Build the release binary
- Install the binary to `~/.local/bin/btsms`
- Install icons to `~/.local/share/icons/`
- Install the desktop entry to `~/.local/share/applications/`

Make sure `~/.local/bin` is in your PATH. Add this to your `~/.bashrc` or `~/.zshrc` if needed:
```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Manual Installation

```bash
# Build
cargo build --release

# Copy binary
mkdir -p ~/.local/bin
cp target/release/btsms ~/.local/bin/

# Copy desktop entry (optional, for application menu)
mkdir -p ~/.local/share/applications
cp assets/btsms.desktop ~/.local/share/applications/
```

## Development

### Setup

1. Clone the repository:
   ```bash
   git clone https://github.com/user/btsms.git
   cd btsms
   ```

2. Install build dependencies (see above)

3. Install Rust toolchain via rustup

### Building

```bash
# Debug build (faster compilation, slower runtime)
cargo build

# Release build (slower compilation, optimized runtime)
cargo build --release
```

### Running

```bash
# Debug mode
cargo run

# Release mode
cargo run --release
```

### Testing

```bash
# Run all tests
cargo test

# Run specific module tests
cargo test phone_normalizer
cargo test vmessage
cargo test database
```

### Code Quality

```bash
# Format code
cargo fmt

# Lint code
cargo clippy
```

## Running the Application

After installation, you can:
- Run `btsms` from the terminal
- Find "BT SMS" in your application menu

The `obexd` service (part of BlueZ) is started automatically via D-Bus activation when the app connects to a device — no manual setup needed.

## Command Line Interface

The application includes a separate CLI binary (`btsms-cli`) for debugging and AI-assisted development. This interface is primarily intended for troubleshooting, automation, and as a helper for AI agent workflows—not as the main user interface.

### Global Options

```bash
btsms-cli --help          # Show help
btsms-cli --json <cmd>    # Output in JSON format (for scripting/AI agents)
```

### Device Commands

```bash
# List paired Bluetooth devices
btsms-cli devices
btsms-cli --json devices              # JSON output

# Connect to a device
btsms-cli connect AA:BB:CC:DD:EE:FF

# Disconnect from a device
btsms-cli disconnect AA:BB:CC:DD:EE:FF
```

### Contact Commands

```bash
# Sync contacts from phone via PBAP
btsms-cli contacts sync
btsms-cli contacts sync --address AA:BB:CC:DD:EE:FF   # Specify device

# List contacts from local database
btsms-cli contacts list
btsms-cli contacts list --limit 100   # Show more contacts

# Search contacts by name or phone number
btsms-cli contacts search "John"
btsms-cli --json contacts search "John"   # JSON output
```

### Message Commands

```bash
# List recent messages from local database
btsms-cli messages list
btsms-cli messages list --limit 50    # Show more messages
btsms-cli --json messages list        # JSON output

# Sync messages from phone to local database
btsms-cli messages sync
btsms-cli messages sync --address AA:BB:CC:DD:EE:FF
btsms-cli messages sync --inbox-only  # Only sync inbox
btsms-cli messages sync --sent-only   # Only sync sent
btsms-cli --json messages sync        # JSON output

# Fetch inbox messages directly from phone via MAP (display only, no database storage)
btsms-cli messages inbox
btsms-cli messages inbox --address AA:BB:CC:DD:EE:FF

# Fetch sent messages directly from phone via MAP (display only, no database storage)
btsms-cli messages sent

# Send an SMS message
btsms-cli messages send "+15551234567" "Hello, World!"
btsms-cli messages send "+15551234567" "Hello!" --address AA:BB:CC:DD:EE:FF
```

### Use Cases

- **Debugging**: Quickly inspect messages, contacts, and device state without launching the GUI
- **Scripting**: Automate SMS sending from shell scripts or cron jobs
- **AI Development**: JSON output mode allows AI agents to interact programmatically
- **Headless Systems**: Send/receive SMS on servers without a display

### Notes

- Device address is auto-detected when not specified (uses first paired phone)
- The `--json` flag must come before the subcommand for JSON output
- Local database commands (`messages list`, `contacts list`) work offline
- Phone commands (`messages sync`, `messages inbox`, `contacts sync`) require active Bluetooth connection
- Use `messages sync` to store messages in the local database; `inbox`/`sent` commands display messages without storing them

## Device Setup

### iPhone

1. Pair your iPhone with your Linux machine via system Bluetooth settings
2. On iPhone: Settings → Bluetooth → [Your PC] → tap the (i) → Toggle "Show Notifications" ON
3. Launch the application

### Android

1. Pair your Android device using system Bluetooth or `bluetoothctl`
2. When prompted on your phone, grant "Allow message access" permission
3. Launch the application

## Configuration

Configuration is stored in `~/.local/share/btsms/config.toml` and is created automatically on first run.

The message database is stored in `~/.local/share/btsms/btsms.db`.

## Architecture

- **Phone Number Normalization**: E.164 format ([phone_normalizer.rs](src/contacts/phone_normalizer.rs))
- **vMessage/BMSG Format**: SMS encoding for MAP ([vmessage.rs](src/bluetooth/vmessage.rs))
- **Database**: SQLite with migrations ([db/mod.rs](src/db/mod.rs))
- **GUI**: GTK4 + Libadwaita ([gui/mod.rs](src/gui/mod.rs))

## License

MIT
