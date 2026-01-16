# Multi-Platform SMS Notification System - Implementation Plan

## Project Overview

Build a Linux application that receives SMS notifications via Bluetooth from both iPhone (primary) and Android (secondary) devices, with integrated contact management for user-friendly phone number display.

### Core Requirements
- **iPhone Support (Primary)**:
  - Receive SMS notifications via ANCS (Apple Notification Center Service)
  - **Send SMS via BLE** (reverse-engineer Windows Phone Link's protocol - NO companion app required)
- **Android Support (Secondary)**: Full SMS send/receive via MAP (Message Access Profile)
- **Contact Management**: Automatic sync from phone using PBAP with contact name resolution
- **User Interface**: GTK4/Libadwaita native Linux application
- **Critical**: All functionality via pure Bluetooth - NO companion apps on phone

### Key Technical Constraints
- **iPhone Uses TWO Protocols**:
  - **BLE/ANCS**: Receive SMS notifications (real-time, 256 byte limit)
  - **Classic Bluetooth MAP**: Send SMS + read message history (RFCOMM/OBEX)
- **iPhone Permission**: User must enable "Settings → Bluetooth → [PC] → Show Notifications"
- **Android**: Uses same MAP protocol (Classic Bluetooth RFCOMM/OBEX)
- **Bluetooth Only**: No companion apps required (confirmed with Windows Phone Link)
- **vMessage Format**: Use BMSG format (similar to vCard) for SMS sending

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     Linux Desktop (Rust App)                     │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │              GTK4/Libadwaita GUI                        │    │
│  │  - Message List View (send + receive)                  │    │
│  │  - Contact Name Resolution                             │    │
│  │  - Message Composition                                 │    │
│  └────────────────────────────────────────────────────────┘    │
│                           ↕                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │           Core Application Logic (Rust)                │    │
│  │  - Contact Manager (vCard, E.164 normalization)        │    │
│  │  - Message Handler (send + receive)                    │    │
│  │  - Database Layer (SQLite)                             │    │
│  └────────────────────────────────────────────────────────┘    │
│         ↕                          ↕                            │
│  ┌──────────────────┐    ┌──────────────────────────┐          │
│  │  ANCS Client     │    │   MAP/PBAP Client        │          │
│  │  (BLE - btleplug)│    │   (Classic BT - zbus)    │          │
│  │  Receive only    │    │   Send + Receive         │          │
│  └──────────────────┘    └──────────────────────────┘          │
│         ↕ BLE                    ↕ D-Bus                        │
│         │                        ↕                              │
│         │              ┌──────────────────┐                     │
│         │              │ BlueZ obexd      │                     │
│         │              │ RFCOMM/OBEX      │                     │
│         │              └──────────────────┘                     │
│         │                        ↕                              │
└─────────┼────────────────────────┼───────────────────────────────┘
          ↓                        ↓
    ┌──────────────────────────────────────┐
    │           iPhone / Android           │
    │                                      │
    │  BLE: ANCS (notifications)           │
    │  Classic BT: MAP (send/receive SMS)  │
    │              PBAP (contacts)         │
    └──────────────────────────────────────┘
```

## Technology Stack

### Core Technologies
- **Language**: Rust (2021 edition)
- **Async Runtime**: Tokio 1.0
- **Database**: SQLite (via sqlx)
- **GUI**: gtk4-rs + libadwaita
- **Serialization**: serde + serde_json

### Bluetooth Stack
- **BLE (ANCS notifications)**: btleplug 0.11+ (iPhone/Android notification receive)
- **Classic Bluetooth (MAP/PBAP)**: zbus 4.0 (D-Bus → BlueZ/obexd)
  - MAP for SMS send/receive (iPhone + Android)
  - PBAP for contact sync (iPhone + Android)
- **Linux Bluetooth**: BlueZ 5.50+ (supports both BLE and Classic Bluetooth)

### Data Formats
- **Contacts**: vCard 3.0 (via vcard4 crate)
- **Phone Numbers**: E.164 normalization
- **SMS Messages (MAP)**: BMSG/vMessage format (vCard-like structure) - iPhone + Android
- **Notifications (ANCS)**: ANCS binary protocol (iPhone real-time notifications only)
- **Internal**: JSON for configuration

## Implementation Phases

### Phase 1: Project Setup & Core Infrastructure
**Goal**: Initialize Rust workspace with all dependencies

#### 1.1 Cargo Workspace Setup
**File**: `Cargo.toml`
```toml
[package]
name = "btsms"
version = "0.1.0"
edition = "2021"

[dependencies]
# Bluetooth
btleplug = "0.11"           # BLE for iPhone ANCS
zbus = "4.0"                # D-Bus for Android MAP/PBAP
uuid = "1.0"                # Service UUIDs

# Async
tokio = { version = "1.0", features = ["full"] }

# GUI
gtk4 = { version = "0.9", features = ["v4_12"] }
libadwaita = { version = "0.7", features = ["v1_5"] }

# Database
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-native-tls"] }

# Data handling
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
vcard4 = "0.5"              # vCard parsing
chrono = "0.4"              # Timestamps
anyhow = "1.0"              # Error handling

# Notifications
notify-rust = "4.0"
```

**System Dependencies**:
```bash
# Debian/Ubuntu
sudo apt-get install libgtk-4-dev libadwaita-1-dev bluez

# Fedora
sudo dnf install gtk4-devel libadwaita-devel bluez

# Arch
sudo pacman -S gtk4 libadwaita bluez
```

#### 1.2 Project Structure
```
btsms/
├── Cargo.toml
├── migrations/
│   ├── 001_initial.sql
│   ├── 002_contacts.sql
│   └── 003_notifications.sql
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── error.rs
│   ├── config.rs
│   ├── bluetooth/
│   │   ├── mod.rs
│   │   ├── ancs_client.rs       # iPhone ANCS
│   │   ├── map_client.rs        # Android MAP
│   │   ├── pbap_client.rs       # Contact sync
│   │   └── bmessage.rs          # bMessage parser
│   ├── contacts/
│   │   ├── mod.rs
│   │   ├── vcard_handler.rs
│   │   ├── phone_normalizer.rs  # E.164
│   │   └── manager.rs
│   ├── db/
│   │   ├── mod.rs
│   │   ├── schema.rs
│   │   ├── contacts.rs
│   │   └── notifications.rs
│   ├── gui/
│   │   ├── mod.rs
│   │   ├── app.rs
│   │   ├── window.rs
│   │   ├── notification_list.rs
│   │   └── widgets/
│   │       ├── notification_row.rs
│   │       └── contact_badge.rs
│   └── notification/
│       ├── mod.rs
│       └── handler.rs
└── README.md
```

### Phase 2: Database Layer & Contact Management

#### 2.1 Database Schema
**File**: `migrations/002_contacts.sql`

```sql
-- Contacts table
CREATE TABLE contacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    display_name TEXT NOT NULL,
    given_name TEXT,
    family_name TEXT,
    vcard_id TEXT UNIQUE NOT NULL,
    source TEXT NOT NULL,  -- 'iphone', 'android', 'local'
    last_modified DATETIME,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    synced_at DATETIME
);

CREATE INDEX idx_contacts_display_name ON contacts(display_name);
CREATE INDEX idx_contacts_source ON contacts(source);

-- Phone numbers with E.164 normalization
CREATE TABLE phone_numbers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    phone_original TEXT NOT NULL,
    phone_normalized TEXT NOT NULL,  -- E.164 format: +15551234567
    phone_type TEXT NOT NULL,        -- CELL, WORK, HOME, OTHER
    is_primary BOOLEAN DEFAULT FALSE,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX idx_phone_normalized ON phone_numbers(phone_normalized);
CREATE INDEX idx_phone_contact_id ON phone_numbers(contact_id);

-- Email addresses
CREATE TABLE email_addresses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    email_type TEXT NOT NULL,
    is_primary BOOLEAN DEFAULT FALSE
);

-- Sync state tracking
CREATE TABLE sync_state (
    id INTEGER PRIMARY KEY,
    device_source TEXT NOT NULL,  -- 'iphone' or 'android'
    last_sync_time DATETIME,
    total_contacts_synced INTEGER DEFAULT 0
);
```

**File**: `migrations/003_notifications.sql`

```sql
-- SMS notifications table
CREATE TABLE sms_notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    notification_uid TEXT UNIQUE NOT NULL,
    device_source TEXT NOT NULL,     -- 'iphone' or 'android'
    sender_number TEXT NOT NULL,
    sender_normalized TEXT NOT NULL, -- E.164 format
    sender_name TEXT,                -- Resolved from contacts
    message_preview TEXT,            -- Max 256 chars for ANCS
    message_full TEXT,               -- Full message (Android only)
    received_at DATETIME NOT NULL,
    read_status BOOLEAN DEFAULT FALSE,
    notification_category TEXT,      -- SMS, MMS, etc.
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_notifications_sender ON sms_notifications(sender_normalized);
CREATE INDEX idx_notifications_received ON sms_notifications(received_at DESC);
CREATE INDEX idx_notifications_unread ON sms_notifications(read_status, received_at DESC);
```

#### 2.2 Phone Number Normalizer
**File**: `src/contacts/phone_normalizer.rs`

Implement E.164 normalization:
- Parse phone numbers (extract digits, country code)
- Normalize to format: `+[CountryCode][Number]` (e.g., `+15551234567`)
- Handle various input formats (dashes, parentheses, spaces)
- Default to US country code (+1) if not specified
- Store both original and normalized versions

#### 2.3 vCard Handler
**File**: `src/contacts/vcard_handler.rs`

Using `vcard4` crate:
- Parse vCard 3.0 format from PBAP
- Extract: display name, phone numbers, emails
- Convert to internal Contact struct
- Handle multiple phone numbers per contact

### Phase 3: Dual-Protocol iPhone Client (Primary)

#### 3.1 ANCS Client (BLE - Real-time Notifications)
**File**: `src/bluetooth/ancs_client.rs`

**Purpose**: Receive real-time SMS notifications from iPhone (push-based, low latency)

**ANCS Service UUIDs**:
```rust
// Service UUID
const ANCS_SERVICE_UUID: &str = "7905F431-B5CE-4E99-A40F-4B1E122D00D0";

// Characteristics
const NOTIFICATION_SOURCE_UUID: &str = "9FBF120D-6301-42D9-8C58-25E699A21DBD";
const CONTROL_POINT_UUID: &str = "69D1D8F3-45E1-49A8-9821-9BBDFDAAD9D9";
const DATA_SOURCE_UUID: &str = "22EAC6E9-24D6-4BB5-BE44-B36ACE7C7BFB";
```

**Use Case**: Real-time notification delivery (supplementary to MAP)

#### 3.2 MAP Client (Classic Bluetooth - Send/Receive SMS)
**File**: `src/bluetooth/map_client.rs` (unified for iPhone + Android)

**Purpose**: Send SMS, receive full message history, mark as read

**MAP Service UUID**:
```rust
// Standard Bluetooth MAP UUID
const MAP_SERVICE_UUID: &str = "00001134-0000-1000-8000-00805f9b34fb";
```

**Protocol**: RFCOMM/OBEX over Classic Bluetooth

**Implementation (via BlueZ/obexd D-Bus)**:
- Connect to MAP session via `org.bluez.obex.Client1`
- Use `org.bluez.obex.MessageAccess1` for operations
- Send SMS using OBEX Put with vMessage/BMSG format
- Receive SMS via OBEX Get (folder navigation: telecom/msg/inbox)

**Core Functionality**:

1. **Connection Management**:
   - Scan for iPhone advertising ANCS service
   - Connect to device via BLE
   - Discover ANCS characteristics
   - Subscribe to Notification Source (notifications)
   - Subscribe to Data Source (notification details)

2. **Notification Reception**:
   - Listen for notification events (8-byte packets)
   - Parse: Event ID, Flags, Category, UID
   - Filter for SMS category (CategoryID = 1)
   - Request full notification attributes via Control Point

3. **Attribute Requests**:
   - Send command to Control Point requesting:
     - Title (sender)
     - Message (body, max 256 bytes)
     - Date (timestamp)
   - Read chunked response from Data Source
   - Reassemble multi-packet messages

4. **Data Structures**:
```rust
pub struct AncsNotification {
    pub uid: u32,
    pub event_type: EventType,  // Added, Modified, Removed
    pub category: Category,      // SMS, Call, Email, etc.
    pub flags: NotificationFlags,
    pub title: Option<String>,
    pub message: Option<String>,
    pub timestamp: Option<chrono::DateTime<Utc>>,
}

pub enum Category {
    Other = 0,
    IncomingCall = 1,
    MissedCall = 2,
    Voicemail = 3,
    Social = 4,
    Schedule = 5,
    Email = 6,
    News = 7,
    HealthAndFitness = 8,
    BusinessAndFinance = 9,
    Location = 10,
    Entertainment = 11,
}

// Note: SMS typically comes as Category::Social or via Messages app
```

**CRITICAL LIMITATION**: ANCS doesn't have a dedicated SMS category. SMS notifications come through as app notifications from the Messages app. Filter by app identifier if needed.

#### 3.3 vMessage/BMSG Format Handler
**File**: `src/bluetooth/vmessage.rs`

**Purpose**: Construct vMessage (BMSG) format for SMS sending via MAP

**vMessage Structure**:
```
BEGIN:BMSG
VERSION:1.0
STATUS:READ
TYPE:SMS_GSM
FOLDER:telecom/msg/outbox
BEGIN:VCARD
VERSION:3.0
FN:Recipient Name
TEL:+1234567890
END:VCARD
BEGIN:BENV
BEGIN:VCARD
VERSION:3.0
FN:Sender Name
TEL:+1987654321
END:VCARD
BEGIN:BBODY
CHARSET:UTF-8
LENGTH:13
BEGIN:MSG
Hello World!
END:MSG
END:BBODY
END:BENV
END:BMSG
```

**Functions**:
```rust
// Create vMessage for SMS
pub fn create_vmessage(recipient: &str, sender: &str, message: &str) -> String;

// Parse received vMessage
pub fn parse_vmessage(content: &str) -> Result<ParsedMessage>;

// Validate vMessage format
pub fn validate_vmessage(content: &str) -> Result<()>;
```

#### 3.4 iPhone Permission Setup
**Critical User Setup Step**:

Before MAP works on iPhone, user must:
1. Pair iPhone with Linux via Bluetooth settings
2. On iPhone: `Settings → Bluetooth → [PC Name] → (i)`
3. **Toggle "Show Notifications" ON**
4. Without this toggle, MAP RFCOMM connection will be rejected

#### 3.5 Error Handling
Handle:
- Bluetooth connection drops
- iPhone going out of range
- Notification parsing errors
- Chunked message reassembly failures
- MTU size limitations

### Phase 4: Unified MAP/PBAP Implementation (iPhone + Android)

**Note**: Both iPhone and Android use the same MAP/PBAP protocols via Classic Bluetooth!

#### 4.1 D-Bus Proxy Definitions
**File**: `src/bluetooth/dbus_proxies.rs`

Use zbus to create proxies for BlueZ D-Bus interfaces:
- `org.bluez.obex.Client1` (session management)
- `org.bluez.obex.MessageAccess1` (MAP operations - iPhone + Android)
- `org.bluez.obex.PhonebookAccess1` (PBAP contact sync - iPhone + Android)

#### 4.2 Unified MAP Client
**File**: `src/bluetooth/map_client.rs` (already covered in Phase 3.2)

**Supports**: iPhone AND Android (same protocol)

**Full SMS Operations**:
- `connect(address: &str, device_type: DeviceType)` - Create MAP session
- `list_inbox_messages()` - Fetch inbox
- `list_sent_messages()` - Fetch sent
- `get_message_content(handle)` - Get full message body
- `send_sms(recipient, text)` - Send SMS via OBEX Put + vMessage
- `mark_as_read(handle)` - Update read status

**Device Detection**:
```rust
pub enum DeviceType {
    iPhone,
    Android,
}

// Detect device type from Bluetooth device info
pub fn detect_device_type(device_name: &str) -> DeviceType;
```

#### 4.3 Unified PBAP Client
**File**: `src/bluetooth/pbap_client.rs`

**Supports**: iPhone AND Android (same protocol)

**Contact Sync Operations**:
- `connect_pbap(address: &str)` - Create PBAP session
- `list_contacts()` - Get all contacts as vCard stream
- `pull_contact(handle)` - Get single vCard
- `sync_contacts_to_db(pool: &SqlitePool, device_source: DeviceType)` - Full sync

**PBAP Protocol Details**:
- Uses OBEX over Classic Bluetooth (RFCOMM)
- vCard 3.0 format (same as vMessage)
- Folder structure: `/telecom/pb.vcf` (main phonebook)
- iPhone limitation: Incremental sync not supported (do full sync)
- Android: May support incremental, but safer to do full sync

### Phase 5: Contact Resolution & Management

#### 5.1 Contact Manager
**File**: `src/contacts/manager.rs`

**Core Functions**:

```rust
pub struct ContactManager {
    db_pool: SqlitePool,
    phone_normalizer: PhoneNormalizer,
}

impl ContactManager {
    // Resolve phone number to contact name
    pub async fn resolve_number(&self, number: &str) -> Option<Contact>;

    // Sync contacts from phone
    pub async fn sync_from_pbap(&self, vcards: Vec<VCard>) -> Result<usize>;

    // Search contacts
    pub async fn search(&self, query: &str) -> Vec<Contact>;

    // Get contact by ID
    pub async fn get_contact(&self, id: i64) -> Option<Contact>;
}
```

**Resolution Flow**:
1. Normalize incoming phone number to E.164
2. Query `phone_numbers` table by `phone_normalized`
3. Join to `contacts` table to get display name
4. Cache results for performance
5. Return contact name or "Unknown" if not found

### Phase 6: GUI Application (GTK4/Libadwaita)

#### 6.1 Main Application Window
**File**: `src/gui/window.rs`

**Layout**:
```
┌──────────────────────────────────────────────────────────┐
│ ☰ SMS Notifications    [iPhone] [Android] [⚙ Settings] │ ← AdwHeaderBar
├──────────────────────────────────────────────────────────┤
│                                                          │
│  Notifications (15 unread)                               │
│                                                          │
│  ┌────────────────────────────────────────────────────┐ │
│  │ 📱 John Doe                         2:30 PM Today  │ │
│  │    Hey, are we still meeting...                    │ │
│  └────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌────────────────────────────────────────────────────┐ │
│  │ 📱 +1-555-987-6543                  1:15 PM Today  │ │
│  │    Your package has been delivered                 │ │
│  └────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌────────────────────────────────────────────────────┐ │
│  │ 📱 Alice Smith                    Yesterday 11:22  │ │
│  │    Thanks for your help!                           │ │
│  └────────────────────────────────────────────────────┘ │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

**Components**:
- `AdwHeaderBar` - Connection status, device switcher, settings
- `GtkListView` - Notification list (scrollable)
- `AdwStatusPage` - Empty state when no notifications
- Device badges (iPhone/Android icons)

#### 6.2 Notification Row Widget
**File**: `src/gui/widgets/notification_row.rs`

**Display**:
- Contact name (or phone number if unknown)
- Message preview (truncated to 80 chars)
- Timestamp (relative: "2 min ago", "Yesterday")
- Device badge (iPhone/Android icon)
- Unread indicator (bold text, background accent)

**Styling**:
```css
.notification-row {
    padding: 12px;
    margin: 4px;
}

.notification-row.unread {
    background: alpha(@accent_bg_color, 0.1);
    font-weight: bold;
}

.notification-row:hover {
    background: alpha(@accent_bg_color, 0.2);
}
```

#### 6.3 Settings/Preferences
**File**: `src/gui/preferences.rs`

**Settings**:
- Bluetooth device selection (iPhone/Android)
- Auto-connect on startup
- Notification filter (all vs unread only)
- Desktop notification enable/disable
- Contact sync interval (manual, 1hr, 6hr, daily)

### Phase 7: Desktop Notifications

#### 7.1 Notification Integration
**File**: `src/notification/handler.rs`

Using `notify-rust`:
- Show desktop notification for new SMS
- Display: Sender name + message preview
- Click action: Open app and focus on notification
- Respect system Do Not Disturb
- Configurable enable/disable

**Example**:
```rust
Notification::new()
    .summary(&format!("SMS from {}", sender_name))
    .body(&message_preview)
    .icon("phone")
    .timeout(Timeout::Milliseconds(5000))
    .show()?;
```

### Phase 8: Testing & Validation

#### 8.1 Unit Tests
- Phone number normalization (E.164)
- vCard parsing (various formats)
- ANCS packet parsing
- bMessage format handling
- Contact resolution logic

#### 8.2 Integration Tests
- ANCS connection to iPhone
- MAP connection to Android
- PBAP contact sync
- Database operations
- GUI rendering

#### 8.3 Manual Testing Checklist

**iPhone (Primary)**:
- [ ] Pair iPhone via Bluetooth settings
- [ ] Connect to ANCS service
- [ ] Receive SMS notification
- [ ] Verify sender name resolution
- [ ] Test with unknown sender (no contact)
- [ ] Verify timestamp accuracy
- [ ] Test notification with emoji
- [ ] Test long message (>256 chars truncation)

**Android (Secondary)**:
- [ ] Pair Android via bluetoothctl
- [ ] Connect to MAP session
- [ ] List inbox messages
- [ ] Read message content
- [ ] Send SMS (verify on phone)
- [ ] Sync contacts via PBAP
- [ ] Verify contact name resolution

**General**:
- [ ] Switch between devices
- [ ] Desktop notification display
- [ ] App startup/shutdown
- [ ] Bluetooth disconnect/reconnect
- [ ] Database persistence across restarts

## Critical Implementation Notes

### iPhone Dual-Protocol Specifics

1. **Two Protocols Required**:
   - **BLE/ANCS**: Real-time notification push (optional, for instant alerts)
   - **Classic BT/MAP**: Full SMS send/receive (required, primary protocol)

2. **Permission Setup** (Critical):
   - Pair iPhone via system Bluetooth
   - On iPhone: `Settings → Bluetooth → [PC] → (i) → Toggle "Show Notifications" ON`
   - Without this toggle, MAP will be rejected
   - This is equivalent to Android's "Allow message access?" dialog

3. **SMS Reception**:
   - **Via MAP**: Full message history, complete content (primary method)
   - **Via ANCS**: Real-time push notifications, 256 byte limit (supplementary)
   - Use both: ANCS for instant alerts, MAP for full content

4. **SMS Sending**:
   - **Via MAP**: OBEX Put with vMessage/BMSG format
   - **Not via ANCS**: ANCS is receive-only
   - iPhone decides: iMessage (blue) vs SMS (green) based on recipient

5. **vMessage Format**:
   - Must use exact BMSG structure (see Phase 3.3)
   - Requires `\r\n` line endings (not `\n`)
   - UTF-8 encoding for message body
   - LENGTH field must match body byte count

### Android MAP Specifics

1. **Permission Dialog**:
   - Android shows: "Allow message access?"
   - User must tap "Allow" on phone
   - Permission persists until unpaired
   - Same as iPhone's "Show Notifications" toggle

2. **Same Protocol as iPhone**:
   - Uses identical MAP/OBEX implementation
   - Uses identical vMessage/BMSG format
   - Same PBAP for contacts
   - **Implementation is unified** between iPhone and Android

3. **Differences from iPhone**:
   - Android may support faster PBAP sync
   - Android provides better error messages
   - Android MAP has been stable longer (since Android 4.4)
   - iPhone MAP support added around iOS 6

4. **Full Features** (Same as iPhone):
   - Bidirectional SMS (send + receive)
   - Full message history access
   - Read status synchronization
   - Mark messages as read

### Contact Sync Strategy

1. **Initial Sync**:
   - Full PBAP sync on first connection
   - Store all contacts in local database
   - Process ~1000 contacts in 30-60 seconds

2. **Incremental Updates**:
   - PBAP doesn't reliably support incremental sync
   - Schedule periodic full re-sync (default: daily)
   - User can trigger manual sync

3. **Multi-Device Handling**:
   - Store device source in contacts table
   - Dedup by normalized phone number
   - Prefer most recently synced contact

### Performance Considerations

1. **Database Optimization**:
   - Index on `phone_normalized` (most frequent query)
   - Index on `received_at DESC` (notification list)
   - Limit notification history to last 30 days
   - Auto-cleanup old notifications

2. **BLE Connection**:
   - ANCS uses minimal power (BLE design)
   - Keep connection alive continuously
   - Reconnect automatically on disconnect

3. **GUI Responsiveness**:
   - Use async for all Bluetooth operations
   - Don't block GTK main thread
   - Use `glib::MainContext::spawn_local()` for UI updates

## User Setup Instructions

### Prerequisites
1. Install BlueZ: `sudo apt install bluez`
2. Ensure user in `bluetooth` group: `sudo usermod -aG bluetooth $USER`
3. Reboot or re-login

### iPhone Setup
1. Enable Bluetooth on iPhone
2. Pair iPhone with Linux via system Bluetooth settings
3. Launch `btsms` application
4. Select iPhone from device list
5. Notifications will appear automatically

### Android Setup
1. Enable Bluetooth on Android
2. Pair Android using `bluetoothctl`:
   ```bash
   bluetoothctl
   scan on
   pair XX:XX:XX:XX:XX:XX
   trust XX:XX:XX:XX:XX:XX
   ```
3. Launch `btsms` application
4. Grant message access permission on Android
5. Select "Sync Contacts" to import phonebook

## Potential Issues & Solutions

### Issue 1: iPhone Not Advertising ANCS
**Symptoms**: Cannot discover iPhone
**Solution**: Ensure iPhone is paired and trusted. Restart Bluetooth on both devices.

### Issue 2: ANCS Notifications Not Received
**Symptoms**: Connected but no notifications appear
**Solution**: Check notification settings on iPhone. Ensure Messages app notifications are enabled.

### Issue 3: Contact Names Not Resolving
**Symptoms**: Shows phone numbers instead of names
**Solution**: Trigger manual contact sync via PBAP. Check PBAP session connection.

### Issue 4: Android MAP Permission Denied
**Symptoms**: MAP session fails
**Solution**: Check Android permission dialog. Re-pair if needed.

### Issue 5: Long Messages Truncated (iPhone)
**Symptoms**: SMS cut off at ~256 characters
**Solution**: This is ANCS limitation. Display truncation indicator ("...") in UI.

## Development Workflow

### Phase Implementation Order
1. ✅ Phase 1: Project setup (Cargo, dependencies)
2. ✅ Phase 2: Database schema, contact management
3. ✅ Phase 3: Unified MAP/PBAP Implementation
   - 3.1: D-Bus proxies for BlueZ
   - 3.2: MAP client (send/receive SMS) - works for both iPhone & Android
   - 3.3: vMessage/BMSG format handler
   - 3.4: PBAP client (contact sync) - works for both iPhone & Android
4. ✅ Phase 4: Contact resolution integration
5. ✅ Phase 5: GUI application (message list, composition, contact display)
6. ✅ Phase 6: Desktop notifications
7. ✅ Phase 7: ANCS client (optional - real-time iPhone notifications)
8. ✅ Phase 8: Testing & validation

### Critical Discovery: iPhone Uses Same Protocol as Android!
- **Both use MAP** (Classic Bluetooth RFCOMM/OBEX) for SMS send/receive
- **Both use PBAP** for contact sync
- **Both use vMessage/BMSG format** for SMS encoding
- **Implementation is unified** - one codebase handles both devices
- **Only difference**: iPhone requires "Show Notifications" toggle in Bluetooth settings

### Testing Strategy
- Start with CLI tool for ANCS testing before GUI
- Test iPhone integration thoroughly (primary)
- Validate Android as secondary
- GUI testing with GTK Inspector (`Ctrl+Shift+D`)

## Success Criteria

**Minimum Viable Product**:
- [ ] Receive iPhone SMS notifications via ANCS
- [ ] **Send SMS to iPhone via BLE** (requires protocol research)
- [ ] Display sender name (from contacts) or number
- [ ] Show message preview (up to 256 chars)
- [ ] Desktop notification integration
- [ ] Contact sync from phone (PBAP/alternative method)
- [ ] Clean GTK4/Libadwaita UI

**Extended Features** (Android):
- [x] Full Android SMS send/receive (MAP)
- [x] Message history browsing
- [x] Mark messages as read
- [x] Contact sync from Android

**Nice-to-Have**:
- [ ] Message search
- [ ] Multiple device support (switch between phones)
- [ ] Export/backup contacts
- [ ] Custom notification sounds

## Resources

### Official Documentation
- Apple ANCS Specification: https://developer.apple.com/library/archive/documentation/CoreBluetooth/Reference/AppleNotificationCenterServiceSpecification/
- Bluetooth MAP Spec: https://www.bluetooth.com/specifications/specs/message-access-profile-1-4/
- BlueZ OBEX API: https://git.kernel.org/pub/scm/bluetooth/bluez.git/tree/doc/obex-api.txt
- BlueZ GATT API: https://git.kernel.org/pub/scm/bluetooth/bluez.git/tree/doc/gatt-api.txt

### Rust Libraries
- btleplug: https://github.com/deviceplug/btleplug
- zbus: https://github.com/dbus2/zbus
- vcard4: https://crates.io/crates/vcard4
- gtk4-rs: https://gtk-rs.org/gtk4-rs/

### Reference Implementations
- btleplug ANCS example: https://github.com/deviceplug/btleplug/tree/master/examples
- nOBEX (MAP/PBAP): https://github.com/nccgroup/nOBEX
- PyPBAP: https://github.com/bmwcarit/pypbap

### Tools
- `bluetoothctl` - Bluetooth pairing
- `dbus-monitor` - D-Bus traffic monitoring
- GTK Inspector - GUI debugging
- `d-feet` - D-Bus debugger (GUI)

## Critical Files to Implement

### Priority 1 (Core Functionality)
1. `src/bluetooth/dbus_proxies.rs` - BlueZ D-Bus interface definitions
2. `src/bluetooth/map_client.rs` - Unified MAP client (iPhone + Android SMS)
3. `src/bluetooth/pbap_client.rs` - Unified PBAP client (iPhone + Android contacts)
4. `src/bluetooth/vmessage.rs` - vMessage/BMSG format handler
5. `src/contacts/phone_normalizer.rs` - E.164 normalization
6. `src/contacts/manager.rs` - Contact resolution
7. `src/db/contacts.rs` - Contact database operations
8. `src/db/notifications.rs` - Message storage

### Priority 2 (Real-time Notifications - Optional)
9. `src/bluetooth/ancs_client.rs` - BLE notification receiver (iPhone only, optional)

### Priority 3 (User Interface)
9. `src/gui/window.rs` - Main window
10. `src/gui/notification_list.rs` - Notification list view
11. `src/gui/widgets/notification_row.rs` - Custom row widget
12. `src/notification/handler.rs` - Desktop notifications

## Verification Plan

### End-to-End Testing

**Scenario 1: iPhone SMS Reception**
1. Send SMS to iPhone from another phone
2. Verify notification appears in Linux app within 5 seconds
3. Verify sender name resolved from contacts
4. Verify desktop notification shown
5. Verify message preview displayed correctly

**Scenario 2: Contact Sync**
1. Connect to iPhone/Android
2. Trigger PBAP sync
3. Verify contacts imported to database
4. Verify E.164 normalization applied
5. Verify contact name resolution works

**Scenario 3: Multi-Device**
1. Pair both iPhone and Android
2. Switch between devices in UI
3. Verify notifications from active device only
4. Verify contacts merged correctly

### Performance Metrics
- Notification latency: <5 seconds from iPhone to Linux
- Contact sync time: <60 seconds for 1000 contacts
- UI responsiveness: <100ms for all interactions
- Memory usage: <100MB idle, <200MB with 500 notifications

## Next Steps After Implementation

1. **User Testing**: Beta test with real iPhone users
2. **Documentation**: Write user manual and troubleshooting guide
3. **Packaging**: Create .deb/.rpm packages, Flatpak
4. **Distribution**: Publish on GitHub, AUR, Flathub
5. **Future Enhancements**:
   - MMS support (if ANCS provides attachments)
   - Multiple phone support
   - Cloud backup of notifications
   - Integration with Linux notification daemons
