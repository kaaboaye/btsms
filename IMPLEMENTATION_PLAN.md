# Linux Phone Link - SMS over Bluetooth Implementation Plan

## Project Overview
Build a Windows Phone Link-like application for Linux that enables SMS messaging over Bluetooth using the MAP (Message Access Profile) protocol. The app will connect to Android phones and allow reading/sending SMS messages.

## Tech Stack Decision
- **Language**: Rust
- **Bluetooth Stack**: BlueZ (via D-Bus, no direct OBEX implementation needed)
- **D-Bus Library**: `zbus` 4.0 (modern async D-Bus bindings)
- **GUI Framework**: `gtk4-rs` with `libadwaita` (native GNOME integration)
- **Async Runtime**: `tokio` 1.0
- **Database**: `sqlx` with SQLite (for message history)
- **Additional**: `serde`, `anyhow`, `notify-rust`

## Architecture
```
Phone (Android) ←→ BlueZ (obexd daemon) ←→ D-Bus ←→ Rust App ←→ GTK4/Adwaita GUI
                                                        ↓
                                                   SQLite DB
```

## Phase 1: Core D-Bus MAP Client

### 1.1 Project Setup
**File**: `Cargo.toml`
```toml
[package]
name = "phone-link-sms"
version = "0.1.0"
edition = "2021"

[dependencies]
zbus = "4.0"
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
chrono = "0.4"
gtk4 = { version = "0.9", features = ["v4_12"] }
libadwaita = { version = "0.7", features = ["v1_5"] }
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-native-tls"] }
notify-rust = "4.0"
```

**System Dependencies** (required for gtk4-rs):
```bash
# Debian/Ubuntu
sudo apt-get install libgtk-4-dev libadwaita-1-dev

# Fedora
sudo dnf install gtk4-devel libadwaita-devel

# Arch
sudo pacman -S gtk4 libadwaita
```

### 1.2 D-Bus Proxy Definitions
**File**: `src/dbus_proxies.rs`

Create zbus proxy traits for:
1. `ObexClient` interface (`org.bluez.obex.Client1`)
   - Method: `create_session(destination: &str, args: HashMap) -> ObjectPath`
   - Method: `remove_session(session: ObjectPath)`

2. `MessageAccess` interface (`org.bluez.obex.MessageAccess1`)
   - Method: `set_folder(name: &str)`
   - Method: `list_messages(folder: &str, filter: HashMap) -> Vec<(ObjectPath, HashMap)>`
   - Method: `get_message(handle: &str, target_file: &str, attachment: bool)`
   - Method: `push_message(source_file: &str, folder: &str, args: HashMap) -> ObjectPath`

**Important**: Use the `#[proxy]` macro from zbus to auto-generate these.

### 1.3 MAP Client Implementation
**File**: `src/map_client.rs`

Implement `MapClient` struct with:

**Connection Management**:
- `new() -> Result<Self>` - Create session D-Bus connection
- `connect(phone_address: &str) -> Result<()>` - Establish MAP session with phone
- `disconnect() -> Result<()>` - Clean up session

**Message Operations**:
- `list_inbox_messages() -> Result<Vec<Message>>` - Fetch inbox
- `list_sent_messages() -> Result<Vec<Message>>` - Fetch sent
- `get_message_content(handle: &str) -> Result<MessageContent>` - Get full message body
- `send_sms(recipient: &str, text: &str) -> Result<()>` - Send SMS
- `mark_as_read(handle: &str) -> Result<()>` - Mark message as read

**Message Data Structures**:
```rust
pub struct Message {
    pub handle: String,
    pub sender: String,
    pub recipient: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub subject: String,
    pub read: bool,
    pub folder: String,
}

pub struct MessageContent {
    pub handle: String,
    pub body: String,
    pub attachments: Vec<String>,
}
```

### 1.4 bMessage Format Handler
**File**: `src/bmessage.rs`

Implement bMessage format parser and generator (MAP protocol uses this format):

**Generator**:
- `create_bmessage(recipient: &str, text: &str) -> String`
  - Format: BEGIN:BMSG...END:BMSG with VCARD and BBODY sections
  - Must follow MAP 1.0 specification

**Parser**:
- `parse_bmessage(content: &str) -> Result<ParsedMessage>`
  - Extract sender, recipient, timestamp, body

**Reference**: The bMessage format is a text-based structure similar to vCard.

### 1.5 Error Handling
**File**: `src/error.rs`

Define custom error types:
```rust
pub enum PhoneLinkError {
    DBusError(zbus::Error),
    NotConnected,
    PhoneNotPaired,
    MessageNotFound,
    InvalidFormat(String),
}
```

## Phase 2: Basic CLI Interface

### 2.1 CLI Application
**File**: `src/main.rs`

Create command-line interface for testing:

**Commands**:
```bash
phone-link-sms connect <MAC_ADDRESS>
phone-link-sms list-inbox
phone-link-sms list-sent
phone-link-sms read <MESSAGE_HANDLE>
phone-link-sms send <PHONE_NUMBER> <MESSAGE_TEXT>
phone-link-sms disconnect
```

**Implementation**:
- Use simple match statement on command args
- Pretty-print message lists as tables
- Show connection status
- Handle errors gracefully

### 2.2 Testing Strategy

**Manual Testing**:
1. Pair phone using `bluetoothctl`
2. Test connection with `phone-link-sms connect`
3. Verify inbox listing works
4. Test sending SMS
5. Verify message appears on phone

**Test Phone Requirements**:
- Android device with MAP support (Android 4.4+)
- Bluetooth enabled
- Grant message access permission when prompted

## Phase 3: GUI Application

### 3.1 GUI Architecture
**File**: `src/gui/mod.rs`

Use GTK4 with libadwaita for native GNOME integration:

**Application Structure**:
```rust
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};
use libadwaita as adw;
use adw::prelude::*;

pub struct PhoneLinkApp {
    application: adw::Application,
    window: adw::ApplicationWindow,
    connection_state: Arc<Mutex<ConnectionState>>,
    map_client: Arc<Mutex<Option<MapClient>>>,
}

#[derive(Clone, Debug)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected { phone_address: String },
    Error(String),
}
```

**Main Window Components**:
- `AdwHeaderBar` - Top header with connection controls
- `AdwNavigationSplitView` - Two-pane layout (conversations | messages)
- `AdwStatusPage` - Connection status/empty states
- `GtkListView` - Conversation list with `GtkSignalListModel`
- `GtkScrolledWindow` - Message thread view
- `GtkEntry` + `GtkButton` - Message composition area

### 3.2 GUI Layouts

**Main Window Structure**:
```
┌──────────────────────────────────────────────────────────┐
│ ☰ Phone Link SMS          [Phone: XX:XX] [⚡Connected]  │ ← AdwHeaderBar
├──────────────┬───────────────────────────────────────────┤
│              │                                           │
│ Conversations│          Message Thread                   │ ← AdwNavigationSplitView
│              │                                           │
│ ┌──────────┐ │ ┌───────────────────────────────────┐  │
│ │📱 Contact│ │ │           Today 2:30 PM           │  │
│ │  1       │ │ └───────────────────────────────────┘  │
│ │  Hello!  │ │                                           │
│ └──────────┘ │     ┌──────────────────────────┐        │
│              │     │ Hey, how are you?        │        │
│ ┌──────────┐ │     └──────────────────────────┘        │ ← Message bubbles
│ │📱 Contact│ │                                           │
│ │  2       │ │  ┌──────────────────────────┐           │
│ │  Thanks! │ │  │ I'm good, thanks!        │           │
│ └──────────┘ │  └──────────────────────────┘           │
│              │                                           │
│              │ ┌─────────────────────────────────────┐ │
│              │ │ Type a message...                   │ │ ← Entry + Send
│              │ └─────────────────────────────────────┘ │
│              │                              [Send] →   │
└──────────────┴───────────────────────────────────────────┘
```

**Files**:
- `src/gui/mod.rs` - Main application setup
- `src/gui/window.rs` - Main window implementation
- `src/gui/header_bar.rs` - Connection controls and status
- `src/gui/conversation_list.rs` - Left sidebar with conversations
- `src/gui/message_view.rs` - Message thread display
- `src/gui/compose_bar.rs` - Message composition area
- `src/gui/widgets/message_row.rs` - Custom message bubble widget
- `src/gui/widgets/conversation_row.rs` - Custom conversation list item

### 3.3 Styling with Libadwaita
**File**: `src/gui/theme.rs`

Use libadwaita's built-in styling system:

**Message Bubbles**:
```rust
// Sent messages - use .accent style class
message_box.add_css_class("accent");
message_box.add_css_class("message-bubble");

// Received messages - use .card style class
message_box.add_css_class("card");
message_box.add_css_class("message-bubble");
```

**Custom CSS** (if needed):
```css
.message-bubble {
    border-radius: 18px;
    padding: 8px 12px;
    margin: 4px 8px;
}

.message-sent {
    margin-left: 48px;
}

.message-received {
    margin-right: 48px;
}
```

**Status Indicators**:
- Use `AdwStatusPage` for connection states
- Use `GtkSpinner` for loading states
- Use `AdwToast` for notifications and feedback

## Phase 4: Message Persistence

### 4.1 Database Schema
**File**: `migrations/001_initial.sql`

```sql
CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    handle TEXT UNIQUE NOT NULL,
    sender TEXT NOT NULL,
    recipient TEXT NOT NULL,
    body TEXT NOT NULL,
    timestamp DATETIME NOT NULL,
    read BOOLEAN NOT NULL DEFAULT 0,
    folder TEXT NOT NULL,
    synced_at DATETIME NOT NULL
);

CREATE INDEX idx_messages_sender ON messages(sender);
CREATE INDEX idx_messages_recipient ON messages(recipient);
CREATE INDEX idx_messages_timestamp ON messages(timestamp DESC);

CREATE TABLE conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_number TEXT UNIQUE NOT NULL,
    last_message_time DATETIME NOT NULL,
    unread_count INTEGER NOT NULL DEFAULT 0
);
```

### 4.2 Database Layer
**File**: `src/db.rs`

Implement database operations:
- `init_db(path: &str) -> Result<SqlitePool>`
- `save_message(pool: &SqlitePool, msg: &Message) -> Result<()>`
- `get_conversation_messages(pool: &SqlitePool, contact: &str) -> Result<Vec<Message>>`
- `get_conversations(pool: &SqlitePool) -> Result<Vec<Conversation>>`
- `mark_as_read(pool: &SqlitePool, handles: Vec<String>) -> Result<()>`

### 4.3 Sync Logic
**File**: `src/sync.rs`

Implement message synchronization:
- Periodic sync every 30 seconds when connected
- Compare local DB with phone's message list
- Download new messages
- Update read status
- Handle deletions

## Phase 5: Notifications

### 5.1 Desktop Notifications
**File**: `src/notifications.rs`

Implement using `notify-rust`:
- Show notification for new messages
- Click notification to open app and conversation
- Display sender and preview text
- Respect system Do Not Disturb settings

## Implementation Approach

Start with Phase 1 and work through each phase sequentially. Each phase builds on the previous one:

1. **Phase 1**: Core D-Bus MAP client - This is the foundation
2. **Phase 2**: CLI interface - Test the core functionality 
3. **Phase 3**: GTK4 GUI - Build the user interface
4. **Phase 4**: Database persistence - Add message storage
5. **Phase 5**: Notifications - Complete the feature set

Test thoroughly at each phase before moving to the next.

## Critical Implementation Notes

### D-Bus Session Requirements
```rust
// obexd runs on session bus, NOT system bus
let connection = Connection::session().await?;
```

### MAP Folder Structure
```
/ (root)
└── telecom/
    └── msg/
        ├── inbox/
        ├── sent/
        ├── deleted/
        └── outbox/
```

### bMessage Format Example
```
BEGIN:BMSG
VERSION:1.0
STATUS:UNREAD
TYPE:SMS_GSM
FOLDER:telecom/msg/outbox
BEGIN:VCARD
VERSION:2.1
N:;;;;
TEL:+1234567890
END:VCARD
BEGIN:BENV
BEGIN:VCARD
VERSION:2.1
N:;;;;
END:VCARD
BEGIN:BBODY
CHARSET:UTF-8
LENGTH:11
BEGIN:MSG
Hello World
END:MSG
END:BBODY
END:BENV
END:BMSG
```

### Phone Pairing (User Setup Instructions)
Users must pair their phone before using the app:
```bash
bluetoothctl
# Inside bluetoothctl:
scan on
# Wait for your phone to appear
pair XX:XX:XX:XX:XX:XX
trust XX:XX:XX:XX:XX:XX
quit
```

### Android Permission Dialog
When first connecting, Android will show a dialog asking:
"Allow [Computer Name] to access messages and call history?"
User must tap "Allow" for MAP to work.

## Testing Checklist

### Unit Tests
- [ ] bMessage parser/generator
- [ ] Message data structure conversions
- [ ] Database operations
- [ ] Error handling

### Integration Tests
- [ ] D-Bus connection establishment
- [ ] Session creation with obexd
- [ ] Message listing
- [ ] Message sending
- [ ] Session cleanup

### Manual Testing
- [ ] Connect to phone
- [ ] List messages from inbox
- [ ] Read message content
- [ ] Send SMS to self
- [ ] Verify SMS appears on phone
- [ ] Disconnect cleanly
- [ ] Reconnect after disconnect
- [ ] Handle phone going out of range
- [ ] Handle phone Bluetooth turned off

## Potential Issues & Solutions

### Issue 1: obexd not running
**Solution**: Check with `systemctl --user status obexd`. If not running, check if `bluez-obexd` package is installed.

### Issue 2: Permission denied on D-Bus
**Solution**: User might need to be in `bluetooth` group: `sudo usermod -aG bluetooth $USER`

### Issue 3: Phone doesn't appear in device list
**Solution**: Ensure phone is paired and trusted in bluetoothctl first.

### Issue 4: MAP session creation fails
**Solution**: 
- Check phone has granted message access permission
- Try unpairing and re-pairing
- Some phones require HFP (hands-free) to be connected first

### Issue 5: bMessage parsing fails
**Solution**: The format is finicky. Ensure exact line endings (\r\n) and no extra whitespace.

## Development Tips

1. **Start with CLI, not GUI**: Get the core MAP functionality working first with a simple CLI before building the GTK4 interface.

2. **Use `RUST_LOG=debug`**: Enable trace logging to see all D-Bus communications:
   ```bash
   RUST_LOG=debug cargo run
   ```

3. **Monitor D-Bus**: Use `dbus-monitor` to watch D-Bus traffic:
   ```bash
   dbus-monitor --session "sender='org.bluez.obex'"
   ```

4. **Test with obexctl**: BlueZ includes `obexctl` CLI tool for testing:
   ```bash
   obexctl
   > connect <MAC_ADDRESS>
   > list-sessions
   ```

5. **Use nOBEX as reference**: The Python nOBEX library (github.com/nccgroup/nOBEX) is an excellent reference for MAP protocol details.

6. **Keep sessions alive**: obexd may timeout idle sessions. Implement keepalive or reconnection logic.

7. **GTK4 threading**: Use `glib::MainContext::default().spawn_local()` for async operations in GTK callbacks. Never block the main thread.

8. **Test with GTK Inspector**: Press `Ctrl+Shift+D` in your running app to open the GTK Inspector for debugging UI issues.

## Success Criteria

All features must be implemented:
- [ ] Connect to paired Android phone
- [ ] List inbox messages
- [ ] Display message content  
- [ ] Send SMS messages
- [ ] Full GTK4/Adwaita GUI with conversation list
- [ ] Message thread display
- [ ] Message composition
- [ ] Disconnect cleanly
- [ ] Message persistence (database)
- [ ] Automatic sync
- [ ] Desktop notifications
- [ ] Multiple conversations support
- [ ] Read status synchronization

### Future Enhancements
- [ ] MMS support (requires additional OBEX work)
- [ ] Contact name resolution from phone book (PBAP)
- [ ] Group messaging
- [ ] Message search
- [ ] Emoji support
- [ ] Attachment support

## Documentation Requirements

Create these files:
1. `README.md` - Project overview, installation, usage
2. `SETUP.md` - Detailed setup instructions for users
3. `DEVELOPMENT.md` - Development environment setup
4. `API.md` - Internal API documentation
5. `TROUBLESHOOTING.md` - Common issues and solutions

## Resources

### Documentation
- BlueZ OBEX API: https://git.kernel.org/pub/scm/bluetooth/bluez.git/tree/doc/obex-api.txt
- MAP Specification: https://www.bluetooth.com/specifications/specs/message-access-profile-1-4/
- zbus Book: https://dbus2.github.io/zbus/
- GTK4 Rust Book: https://gtk-rs.org/gtk4-rs/stable/latest/book/
- Libadwaita Docs: https://world.pages.gitlab.gnome.org/Rust/libadwaita-rs/stable/latest/docs/libadwaita/

### Reference Implementations
- nOBEX (Python): https://github.com/nccgroup/nOBEX
- blurz (Rust, outdated): https://github.com/szeged/blurz
- KDE Connect (C++/Qt): https://invent.kde.org/network/kdeconnect-kde

### Tools
- `bluetoothctl` - Bluetooth device management
- `obexctl` - OBEX session management
- `dbus-monitor` - D-Bus traffic monitoring
- `d-feet` - D-Bus debugger (GUI)
- GTK Inspector (`Ctrl+Shift+D` in app) - GTK widget debugging

## Implementation Instructions

Implement all phases in order. Each phase is required and builds on the previous:

1. **Phase 1**: Core D-Bus MAP client - Foundation
2. **Phase 2**: CLI interface - Validation  
3. **Phase 3**: GTK4 GUI - User interface
4. **Phase 4**: Database - Persistence
5. **Phase 5**: Notifications - Completion

Test thoroughly after each phase. Refer to the testing checklist and troubleshooting sections as needed.
