# BT SMS

Send and receive SMS from your phone on Linux over Bluetooth — a small Phone Link
alternative. GTK4/libadwaita app plus a CLI.

> **Note**: This project was vibe coded with AI assistance. Use at your own risk.

## Features

- **iPhone and Android** — messages over MAP, contacts over PBAP
- **Instant on iPhone** — ANCS tells the app the moment a message arrives; Android is polled
- **Contact names** from the phone's address book
- **Local history** in SQLite, grouped into conversations
- **Desktop notifications**, clickable links, adaptive layout for narrow windows
- **Automatic reconnect** when the phone goes out of range and comes back
- **CLI** with JSON output, a `doctor` command for diagnosing Bluetooth problems and a
  `listen` mode
- **Logs** written to `~/.local/state/btsms/logs/` for debugging

## Limitations

These come from the Bluetooth profiles, not from this app:

- **No full history**: phones only expose recent messages over MAP.
- **iPhone**: messages you send *from the phone* are not visible (iOS keeps the MAP "sent"
  folder empty). Messages sent from this app are stored locally.
- **Text only**: no MMS, photos or group chats. iMessage vs SMS is decided by the iPhone.
- Messages arrive only while the phone is connected.
- An iPhone accepts one message-access session at a time: while the GUI is connected,
  `btsms-cli` commands that talk to the phone fail with "connection refused". Use
  `btsms-cli listen` *instead of* the GUI, not next to it.

## Requirements

- BlueZ with **obexd** (MAP/PBAP), GTK ≥ 4.12, libadwaita ≥ 1.5.

| Distro | Runtime packages |
|--------|------------------|
| Debian / Ubuntu (24.04+) | `sudo apt install bluez bluez-obexd libgtk-4-1 libadwaita-1-0` |
| Fedora | `sudo dnf install bluez bluez-obexd gtk4 libadwaita` |
| Arch / Manjaro | `sudo pacman -S bluez bluez-obex gtk4 libadwaita` |

To build you also need Rust ([rustup](https://rustup.rs/)), a C compiler, `pkg-config` and the
GTK/libadwaita development files:

| Distro | Build packages |
|--------|----------------|
| Debian / Ubuntu | `sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev` |
| Fedora | `sudo dnf install gcc pkg-config gtk4-devel libadwaita-devel` |
| Arch / Manjaro | `sudo pacman -S base-devel pkg-config gtk4 libadwaita` |

SQLite is bundled; no system library is needed.

## Install

```bash
git clone https://github.com/kaaboaye/btsms.git
cd btsms
./install.sh            # installs to ~/.local (override with PREFIX=...)
```

This installs `btsms` and `btsms-cli` to `~/.local/bin`, the icons and the desktop entry.

## Phone setup

1. Pair the phone in your system Bluetooth settings.
2. **iPhone**: Settings → Bluetooth → (i) next to this computer → enable **Show
   Notifications** (messages) and **Sync Contacts** (names).
3. **Android**: accept the "message access" and "contact sharing" prompts on the phone
   (or enable them in the phone's Bluetooth settings for this computer).
4. Start BT SMS. It connects to the last used phone automatically; use the menu → *Connect
   Phone…* to choose another.

`obexd` is started on demand through D-Bus; no manual setup is needed.

## Troubleshooting

Run the diagnosis first — it checks BlueZ, obexd, MAP and ANCS step by step:

```bash
btsms-cli doctor
```

Logs: `~/.local/state/btsms/logs/` (one file per day, 7 days kept). Message texts are never
written to the logs at the default level. More detail:

```bash
btsms -v                          # GUI, debug output on the terminal too
btsms-cli -vv messages sync       # CLI, debug output
BTSMS_LOG=btsms=trace,zbus=debug btsms   # everything, including D-Bus traffic
```

The GUI's *About → Troubleshooting* section has version and path information to paste into
bug reports, and *Preferences* has a button to open the log folder.

Numbers you type are sent exactly as typed; the phone applies its own dialling rules. To
group conversations and match contact names, numbers without a country code are read using
the region from your locale (`LANG=pl_PL.UTF-8` → Poland). If threads for the same person
are split, set *Preferences → Phone Number Region*.

## Command line

`btsms-cli` uses the same database as the app. `--json` gives machine-readable output, `-v`
more logging, `--database PATH` another database.

```bash
btsms-cli devices                         # paired devices
btsms-cli doctor [-a ADDR]                # diagnose Bluetooth/MAP/ANCS
btsms-cli listen [-a ADDR]                # stay connected, print messages as they arrive
btsms-cli paths                           # config, database and log locations

btsms-cli conversations                   # threads from the local database
btsms-cli messages list [--peer +48…] [-l 50]
btsms-cli messages sync [--inbox-only | --sent-only]
btsms-cli messages inbox [--full]         # what the phone has now (nothing is stored)
btsms-cli messages sent
btsms-cli messages send "+48 512 345 678" "Hello!" [--dry-run]

btsms-cli contacts sync
btsms-cli contacts list [-l 100]
btsms-cli contacts search "Jan"

btsms-cli connect AA:BB:CC:DD:EE:FF
btsms-cli disconnect AA:BB:CC:DD:EE:FF
```

The phone is auto-detected when `--address` is omitted (a connected phone is preferred).

## Files

| What | Where |
|------|-------|
| Config | `~/.local/share/btsms/config.toml` |
| Database | `~/.local/share/btsms/btsms.db` (older versions also used `messages.db`; it is merged automatically and renamed to `messages.db.merged`) |
| Logs | `~/.local/state/btsms/logs/` |

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo run                 # GUI
cargo run --bin btsms-cli -- doctor
```

Architecture:

- `src/bluetooth/` — BlueZ D-Bus clients: MAP/PBAP via obexd (`obex.rs`, `map_client.rs`,
  `pbap_client.rs`), ANCS over GATT (`ancs.rs` protocol, `ancs_client.rs` transport),
  bMessage format (`vmessage.rs`)
- `src/service.rs` — the long-running phone link: reconnects, ANCS-triggered sync, polling,
  sending
- `src/sync/` — MAP → database import
- `src/contacts/` — vCard parsing, phone number normalisation, contact storage
- `src/db/` — SQLite schema, migrations, queries
- `src/gui/` — GTK4/libadwaita front end
- `docs/protocols.md` — how MAP, PBAP and ANCS are used, with iPhone quirks

## License

MIT
