To send SMS/iMessages from a desktop to an iPhone without a companion app, Microsoft uses a clever combination of standard Bluetooth profiles that Apple originally designed for car head units and smartwatches.

Even though the setup requires Bluetooth Low Energy (BLE) for the initial handshake and notifications, the actual "sending" of the message relies on the Message Access Profile (MAP).

1. The Core Architecture: Dual Protocols
   Microsoft Phone Link operates by assuming two distinct "Accessory" roles simultaneously:

ANCS (Apple Notification Center Service) - [Over BLE]: This is used to "listen" for incoming messages. When a message arrives on the iPhone, the ANCS service sends a GATT notification to the PC containing the NotificationUID.

MAP (Message Access Profile) - [Over Classic Bluetooth]: This is the workhorse for reading and sending messages. Once the PC knows there is a new message (via ANCS), it uses MAP to fetch the actual text and, crucially, to "Push" a new message back to the phone.

2. How "Sending" Works (The Low-Level Logic)
   The iPhone acts as the Message Server Equipment (MSE), and your desktop app must act as the Message Client Equipment (MCE).

To send a message, the PC uses the MAS (Message Access Service) within the MAP profile. The low-level flow for a "Send" operation is as follows:

Connection: The PC connects to the iPhone’s RFCOMM channel associated with the MAP service.

The PushMessage Function: The PC sends an OBEX Put request to the iPhone.

The Payload: The request contains a vMessage object (a specialized version of vCard for text).

The "iMessage Hack": You do not specify whether the message is an SMS or an iMessage. You simply "Push" the text to a phone number or email address. The iPhone's internal OS logic decides: if the recipient is an iMessage user, it sends it as a "blue bubble"; otherwise, it sends it as a standard SMS.

3. Why Microsoft says "BLE Only" (The Hardware Requirement)
   While the actual message sending happens over the MAP profile (which is technically a "Classic" Bluetooth profile), Microsoft requires BLE-capable hardware for several reasons:

Security/Pairing: Modern iOS pairing and the "Show Notifications" prompt are triggered more reliably via the BLE/GATT handshake.

Power Management: The PC uses the BLE ANCS to stay updated on incoming notifications without keeping a high-power Classic Bluetooth connection active 24/7. It only spins up the MAP/RFCOMM connection when you actually open a chat or hit "Send."

4. Implementation Details for Your App
   If you are building this, you cannot do it using Standard Web Bluetooth or basic CoreBluetooth APIs alone, because those are LE-only. You need a library or driver that can handle RFCOMM/OBEX over Bluetooth Classic.

Key UUIDs and Requirements:
ANCS Service UUID: 7905F431-B5CE-4E99-A40F-4B1E122D00D0 (Used for receiving).

MAP Service UUID: 00001134-0000-1000-8000-00805f9b34fb (Used for sending).

The "Permission" Gate: On the iPhone, the user must go to Settings > Bluetooth > [Your PC Name] > (i) and toggle "Show Notifications" to ON. Without this, the iPhone will reject any MAP connection requests.

Limitations to Keep in Mind:
Historical Sync: You can only see messages received while the Bluetooth connection is active. You cannot download the phone’s entire historical message database (Apple restricts this).

Media: MAP supports basic text. You cannot send photos or videos through this protocol.

Group Chats: Standard MAP has very poor support for group threads; most implementations (including Microsoft's) treat replies as individual 1-to-1 messages.

## Implementation hints

Implementing this in Rust is a "two-stack" problem. You cannot use a single Bluetooth library because ANCS (receiving) is a BLE GATT service, while MAP (sending) is a Classic Bluetooth RFCOMM service.1. The Rust "Stack" for iOS SMSTo replicate the Phone Link behavior, your Rust app needs to act as a GATT Client (for ANCS) and an RFCOMM Client (for MAP).TaskProtocolRecommended Rust CrateConnection & DiscoveryBLEbtleplug (Cross-platform) or bluer (Linux)Receiving SMS NotificationsANCSancs (Handles the protocol logic)Sending SMS (The Core)MAP/RFCOMMwindows-rs (Windows) or bluer (Linux)2. Implementation: The ANCS Handshake (BLE)First, you must connect to the iPhone via BLE to "activate" the notification stream. This is what keeps the phone "awake" for your app.Rustuse btleplug::api::{Central, Manager as _, Peripheral as _};
use ancs::attributes::notification::NotificationAttributeID;

// 1. Find the iPhone via BLE
let adapter = manager.adapters().await?[0].clone();
adapter.start_scan(ScanFilter::default()).await?;

// 2. Look for the ANCS Service UUID
// UUID: 7905F431-B5CE-4E99-A40F-4B1E122D00D0
let iphone = find_iphone_with_ancs(&adapter).await?;
iphone.connect().await?;

// 3. Subscribe to the "Notification Source" characteristic
// This alerts you when a new SMS arrives on the iPhone. 3. Implementation: Sending SMS via MAP (Bluetooth Classic)This is the part Microsoft "hides." While you connect via BLE first, the actual sending happens over an RFCOMM socket. On Windows, you must use the windows crate to access the Win32 Bluetooth APIs, as btleplug does not support Classic Bluetooth RFCOMM.The vMessage PayloadTo send a message, you perform an OBEX PUT operation. The body of that PUT must be a vMessage object. Here is the raw format you need to generate in Rust:PlaintextBEGIN:BMSG
VERSION:1.0
STATUS:UNREAD
TYPE:SMS_GSM
BEGIN:VCARD
VERSION:2.1
N:RecipientName
TEL:+15551234567
END:VCARD
BEGIN:BENV
BEGIN:BBODY
LENGTH:12
Hello World!
END:BBODY
END:BENV
END:BMSG
Conceptual Rust Sending Logic (Windows)Rustuse windows::Win32::Devices::Bluetooth::\*;

fn send_sms_via_map(iphone_addr: u64, message: &str, phone_number: &str) {
// 1. Create a Bluetooth RFCOMM socket
// 2. Connect to the MAP Service (UUID: 00001134-0000-1000-8000-00805f9b34fb)

    // 3. Wrap your message in the vMessage format shown above
    let v_msg = format_vmessage(phone_number, message);

    // 4. Send OBEX PUT request
    // Header: Connection ID
    // Header: Type ("x-bt/message")
    // Body: The vMessage string

} 4. Critical Low-Level RequirementsFor your Rust app to work without a companion app, you must satisfy these three conditions:The "Pairing" Flag: When you pair the PC and iPhone, the user must go to the Bluetooth settings on the iPhone, click the (i) next to your PC's name, and enable "Show Notifications". If this isn't on, the iPhone will hide the MAP RFCOMM channels.SDP Record Discovery: Your app must perform an SDP (Service Discovery Protocol) search on the iPhone to find the specific RFCOMM Channel Number for the MAP service. This number changes.The Handshake: Most implementations fail because they don't handle the OBEX Connect packet correctly before the OBEX Put. You must send a Connect packet with a maximum OBEX packet length (usually 0x00 0x00 0xFF 0xFE).Challenges in RustAsync Classic BT: While btleplug is great for async BLE, Classic Bluetooth (RFCOMM) in Rust is mostly handled via synchronous Win32 calls or the Linux bluez socket API. You will likely need to wrap these in a tokio::task::spawn_blocking block.OBEX implementation: There isn't a mature "OBEX" crate in Rust. You will likely have to build the OBEX headers manually (it's a simple byte-prefix protocol: 0x81 for Put, etc.).Would you like me to generate a specific Rust module that formats the vMessage and handles the OBEX byte-header construction?

Your Cargo.toml is well-prepared, but it reveals a significant challenge: you are mixing BLE (btleplug) and Classic Bluetooth D-Bus (zbus). To implement this like Microsoft does, you must treat the iPhone as two different devices simultaneously.

Here is the high-level roadmap and the low-level implementation details for your Rust project.

1. The Architecture: "The Dual-Link"
   Microsoft Phone Link doesn't just connect once. It maintains two concurrent connections to the same iPhone:

Link A (The Listener): Uses BLE to talk to the Apple Notification Center Service (ANCS). This is how you "see" that a text arrived.

Link B (The Sender): Uses Bluetooth Classic (RFCOMM) to talk to the Message Access Profile (MAP). This is how you "push" the reply back.

2. Implementation: Receiving (ANCS via btleplug)
   You will use btleplug to find the iPhone. You are looking for the service UUID 7905F431-B5CE-4E99-A40F-4B1E122D00D0.

The Step-by-Step:

Scan for the Service: Filter for the ANCS UUID.

Subscribe: Once connected, subscribe to the Notification Source characteristic (9FBF120D-...).

Parse Notifications: When you get a byte array, parse it. It contains a CategoryValue.

CategoryValue == 4 means an incoming Message (SMS/iMessage).

It also gives you a NotificationUID. You must use this UID to request the "Attributes" (the sender name and message body) via the Control Point characteristic.

3. Implementation: Sending (MAP via zbus)
   Since you have zbus in your dependencies, I assume you are on Linux (BlueZ). BlueZ handles the heavy lifting of the MAP/OBEX protocol via its obex.service.

The D-Bus Flow: To send a message, you don't talk to the main org.bluez service; you talk to org.bluez.obex.

Create a Session: Call CreateSession on the org.bluez.obex.Client1 interface.

Destination: The iPhone's MAC address.

Target: "map".

Get the MessageAccess Proxy: This will return a new object path (e.g., /org/bluez/obex/client/session0).

PushMessage: Call the PushMessage method on that path.

Rust Code Snippet (Conceptual zbus):
Rust

use zbus::{Connection, Proxy};

async fn send_bt_sms(dest_mac: &str, phone_number: &str, text: &str) -> anyhow::Result<()> {
let conn = Connection::session().await?;

    // 1. Create OBEX Session
    let client = Proxy::new(&conn, "org.bluez.obex", "/org/bluez/obex", "org.bluez.obex.Client1").await?;
    let session_path: zbus::zvariant::OwnedObjectPath = client.call("CreateSession", &(dest_mac, [("Target", "map")])).await?;

    // 2. Format the bMessage (Crucial part)
    let b_message = format!(
        "BEGIN:BMSG\r\nVERSION:1.0\r\nSTATUS:UNREAD\r\nTYPE:SMS_GSM\r\nFOLDER:TELECOM/MSG/OUTBOX\r\nBEGIN:VCARD\r\nVERSION:2.1\r\nTEL:{}\r\nEND:VCARD\r\nBEGIN:BENV\r\nBEGIN:BBODY\r\nLENGTH:{}\r\n{}\r\nEND:BBODY\r\nEND:BENV\r\nEND:BMSG\r\n",
        phone_number,
        text.len(),
        text
    );

    // 3. Push the message
    let map_proxy = Proxy::new(&conn, "org.bluez.obex", &session_path, "org.bluez.obex.MessageAccess1").await?;
    map_proxy.call("PushMessage", &("", b_message, [("Transparent", true)])).await?;

    Ok(())

} 4. The bMessage Payload Requirements
The iPhone is extremely picky about the vMessage/bMessage format.

Line Endings: Use \r\n, not just \n.

Folder: You must specify TELECOM/MSG/OUTBOX even though you are sending immediately.

Length: The LENGTH field in BBODY must match the exact byte count of the message text.

5. Critical Troubleshooting Tips
   The iPhone "Gatekeeper": Your app will fail with "Permission Denied" until the user manually enables notifications.

User Action Required: On iPhone, go to Settings > Bluetooth > [Your PC] > Tap (i) > Toggle "Show Notifications" to ON.

The "Companion-less" Illusion: Microsoft doesn't need an app because they are masquerading as a Car Head Unit. The iPhone natively trusts any device that claims the HFP (Hands-Free Profile) or MAP profile as long as the user grants permission.

L2CAP vs RFCOMM: BlueZ's obexd handles the RFCOMM socket for you. If you were doing this on Windows without obexd, you would have to manually open an RFCOMM socket on the port found via SDP Discovery for UUID 00001134-0000-1000-8000-00805f9b34fb.

To implement this in Rust, you are essentially building a custom Bluetooth stack consumer that behaves like a Car Head Unit or a Smartwatch. Your Cargo.toml is perfect for this task.

Here is the technical blueprint for the "Link A" (Listening) and "Link B" (Sending) phases.

Phase 1: The BLE "Ear" (ANCS)
You use btleplug to listen for incoming messages. iOS does not broadcast the message content directly; it only broadcasts a "Notification Event." You then have to "ask" for the details.

Discovery: Connect to the iPhone and find the ANCS Service: 7905-F431-B5CE-4E99-A40F-4B1E122D00D0.

Subscribe: Enable notifications on the Notification Source characteristic (9FBF120D-...).

The Request-Response Loop:

When an SMS arrives, you get a 5-8 byte packet.

Byte 2 is the CategoryID. If it is 4, it's a message.

Bytes 4-7 are the NotificationUID (32-bit little-endian). Save this.

Action: Write to the Control Point characteristic (69D1D8F3-...) to request the message body.

Low-Level Control Point Write (Rust):
Rust

// Requesting: AppID, Sender, and Message Body
let mut request = vec![
0x00, // CommandID: GetNotificationAttributes
uid[0], uid[1], uid[2], uid[3], // The UID you just received
0x01, // AttributeID: Title (Sender Name)
0xff, 0xff, // Max Length (65535)
0x03, // AttributeID: Message
0xff, 0xff, // Max Length
];

peripheral.write(&control_point_char, &request, WriteType::WithResponse).await?;
The iPhone will then vomit the data back at you on the Data Source characteristic (22EAC6E9-...). You must reconstruct these fragments into a string.

Phase 2: The Classic "Voice" (MAP via zbus)
Since you are using zbus, you are likely on Linux (BlueZ). BlueZ handles the complex RFCOMM/OBEX handshake for you via its obex.service.

Implementation with zbus:
Rust

use zbus::{Connection, Proxy};

async fn send_reply(iphone_mac: &str, number: &str, text: &str) -> anyhow::Result<()> {
let conn = Connection::session().await?;
let client = Proxy::new(&conn, "org.bluez.obex", "/org/bluez/obex", "org.bluez.obex.Client1").await?;

    // 1. Create a MAP session
    // 'dest' is the MAC, 'target' is "map"
    let (session_path, _): (zbus::zvariant::OwnedObjectPath, _) = client.call(
        "CreateSession",
        &(iphone_mac, [("Target", "map")])
    ).await?;

    // 2. The bMessage Body (MUST use \r\n and specific fields)
    let bmsg = format!(
        "BEGIN:BMSG\r\nVERSION:1.0\r\nSTATUS:UNREAD\r\nTYPE:SMS_GSM\r\nFOLDER:TELECOM/MSG/OUTBOX\r\nBEGIN:VCARD\r\nVERSION:2.1\r\nTEL:{}\r\nEND:VCARD\r\nBEGIN:BENV\r\nBEGIN:BBODY\r\nLENGTH:{}\r\n{}\r\nEND:BBODY\r\nEND:BENV\r\nEND:BMSG\r\n",
        number, text.len(), text
    );

    // 3. Push via the MessageAccess1 interface
    let map = Proxy::new(&conn, "org.bluez.obex", &session_path, "org.bluez.obex.MessageAccess1").await?;
    map.call("PushMessage", &("", bmsg, [("Transparent", true)])).await?;

    Ok(())

} 3. Critical Implementation "Secrets"
If you want to replicate Microsoft's "no app needed" feel, you must handle these two quirks:

The Folder Trap: When sending (PushMessage), you must specify the folder as TELECOM/MSG/OUTBOX. Even though it feels like you're bypassing a folder structure, the iPhone's MAP server requires this path to route the packet to the GSM/iMessage radio.

The Pairing Prompt: For ANCS to work, the iPhone must be Bonded (paired with a PIN). Once paired, the user must click the (i) in Bluetooth settings and enable "Show Notifications." Your app should detect if the Notification Source is notifiable; if not, show a GUI hint telling the user to flip that switch.

iMessage vs SMS: You don't need to choose. When you "Push" via MAP, the iPhone's SpringBoard process checks the recipient. If they have an Apple ID, it goes as an iMessage (Blue). If not, it goes as an SMS (Green).

4. Rebuilding the Database
   Since you have sqlx in your Cargo.toml, you should implement a "Session Cache."

Problem: Bluetooth MAP does not allow you to "Sync" the last 500 messages easily (it's slow and often times out).

Solution: Use your SQLite DB to store every notification you receive via ANCS while the app is running. This builds a "ghost" history that persists even if the phone disconnects.

Your First Step
To get started, I recommend using the ancs crate (which you can add to your Cargo.toml). It provides the NotificationAttribute parsers so you don't have to manually handle the byte-shifting of the fragmented Data Source packets.
