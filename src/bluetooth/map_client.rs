//! MAP (Message Access Profile) client on top of BlueZ obexd.

use crate::bluetooth::dbus_proxies::{
    DictArg, MessageAccessProxy, MessageProxy, PropMap, prop_bool, prop_string, prop_u64,
};
use crate::bluetooth::obex::ObexSession;
use crate::bluetooth::vmessage::{self, BMessage};
use crate::error::{BtsmsError, Result, dbus_error_name, is_unknown_object};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::Mutex;
use zbus::{OwnedObjectPath, Variant};

const MESSAGE_GET_TIMEOUT: Duration = Duration::from_secs(30);
const PUSH_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for the MAP interface after CreateSession (iOS can be slow to accept).
const SESSION_READY_TIMEOUT: Duration = Duration::from_secs(15);
/// Phones cut the listing subject at 256 characters (MAP spec maximum).
const SUBJECT_LIMIT: usize = 250;
/// Upper bound on messages per listing (BlueZ default is 1024, the MAP maximum 65535).
const LIST_MAX_COUNT: u16 = 1024;

/// MAP folders under `telecom/msg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapFolder {
    Inbox,
    Sent,
    Outbox,
    Deleted,
}

impl MapFolder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Sent => "sent",
            Self::Outbox => "outbox",
            Self::Deleted => "deleted",
        }
    }
}

impl std::fmt::Display for MapFolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Metadata from a MAP message listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapMessage {
    /// Object path of the message on the obexd side (needed to download it).
    pub path: String,
    /// MAP handle (the last path component without the `message` prefix).
    pub handle: String,
    pub folder: MapFolder,
    /// First part of the text (phones cut it at 256 characters or less).
    pub subject: String,
    /// `YYYYMMDDTHHMMSS[±HHMM]`
    pub timestamp: String,
    /// Display name of the sender as the phone knows it.
    pub sender: String,
    /// Phone number (or e-mail) of the sender.
    pub sender_address: Option<String>,
    pub recipient: Option<String>,
    pub recipient_address: Option<String>,
    /// "sms-gsm", "sms-cdma", "email", "mms", "im"
    pub message_type: String,
    /// Size of the full text in bytes.
    pub size: u64,
    pub read: bool,
}

impl MapMessage {
    /// True when the listing's subject already holds the whole text, so no download is needed.
    ///
    /// Phones disagree on `Size`: Android counts UTF-16 units, others bytes. Comparing UTF-16
    /// units is safe for both (for non-ASCII text it can only cause an extra download, never
    /// a truncated message). Subjects at the 256-character listing limit are never trusted.
    pub fn subject_is_complete(&self) -> bool {
        let units = self.subject.encode_utf16().count() as u64;
        self.size > 0 && units >= self.size && self.subject.chars().count() < SUBJECT_LIMIT
    }
}

/// MAP client. Cheap operations are serialised internally: MAP has a single "current
/// folder" per session, so concurrent folder changes would corrupt each other.
pub struct MapClient {
    device_address: String,
    session: Option<ObexSession>,
    op_lock: Mutex<()>,
}

impl MapClient {
    pub fn new(device_address: String) -> Self {
        Self {
            device_address,
            session: None,
            op_lock: Mutex::new(()),
        }
    }

    pub fn device_address(&self) -> &str {
        &self.device_address
    }

    pub fn is_connected(&self) -> bool {
        self.session.is_some()
    }

    /// Open the MAP session and wait until the phone accepts it.
    pub async fn connect(&mut self) -> Result<()> {
        if self.session.is_some() {
            return Ok(());
        }
        let session = ObexSession::open(&self.device_address, "map").await?;
        wait_for_session_ready(&session).await?;
        self.session = Some(session);
        Ok(())
    }

    /// Close the MAP session.
    pub async fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            session.close().await;
        }
    }

    fn session(&self) -> Result<&ObexSession> {
        self.session.as_ref().ok_or(BtsmsError::NotConnected)
    }

    async fn access(&self) -> Result<MessageAccessProxy<'static>> {
        let session = self.session()?;
        Ok(MessageAccessProxy::builder(session.connection())
            .path(session.path().clone())
            .build()
            .await?)
    }

    /// Cheap call that fails when the phone has gone away; used as a health check.
    pub async fn ping(&self) -> Result<()> {
        let _guard = self.op_lock.lock().await;
        let access = self.access().await?;
        access.set_folder("/").await.map_err(map_error)?;
        Ok(())
    }

    /// List the messages in a folder.
    pub async fn list_messages(&self, folder: MapFolder) -> Result<Vec<MapMessage>> {
        let _guard = self.op_lock.lock().await;
        let access = self.access().await?;
        go_to(&access, &format!("telecom/msg/{folder}")).await?;

        let mut filter: DictArg<'_> = HashMap::new();
        filter.insert("MaxCount", Variant::from(LIST_MAX_COUNT));
        let started = std::time::Instant::now();
        let listing = access.list_messages("", filter).await.map_err(map_error)?;

        let mut messages: Vec<MapMessage> = listing
            .into_iter()
            .filter_map(|(path, props)| parse_listing_entry(&path, folder, &props))
            .collect();
        messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        tracing::debug!(
            %folder,
            count = messages.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "listed MAP messages"
        );
        Ok(messages)
    }

    pub async fn list_inbox_messages(&self) -> Result<Vec<MapMessage>> {
        self.list_messages(MapFolder::Inbox).await
    }

    pub async fn list_sent_messages(&self) -> Result<Vec<MapMessage>> {
        self.list_messages(MapFolder::Sent).await
    }

    /// Download and decode a full message.
    pub async fn get_message(&self, message: &MapMessage) -> Result<BMessage> {
        let _guard = self.op_lock.lock().await;
        let session = self.session()?;
        let proxy = MessageProxy::builder(session.connection())
            .path(OwnedObjectPath::try_from(message.path.as_str())?)
            .build()
            .await?;

        let watcher = session.watch_transfers().await?;
        let (transfer, props) = proxy.get("", false).await.map_err(map_error)?;
        let file = prop_string(&props, "Filename").map(std::path::PathBuf::from);
        let _cleanup = file.clone().map(RemoveOnDrop);

        watcher
            .wait(&transfer, file.as_deref(), MESSAGE_GET_TIMEOUT)
            .await?;

        let file = file.ok_or_else(|| {
            BtsmsError::Bluetooth("obexd did not report where it stored the message".into())
        })?;
        let raw = tokio::fs::read(&file).await?;
        let content = String::from_utf8_lossy(&raw);
        tracing::trace!(
            handle = message.handle,
            bytes = raw.len(),
            "downloaded bMessage"
        );
        vmessage::parse_bmessage(&content)
    }

    /// Send an SMS. `recipient` must already be normalised.
    pub async fn send_sms(&self, recipient: &str, text: &str) -> Result<()> {
        let _guard = self.op_lock.lock().await;
        let session = self.session()?;
        let access = self.access().await?;

        let bmessage = vmessage::create_bmessage(recipient, text);
        let tmp = tempfile::Builder::new()
            .prefix("btsms-send-")
            .suffix(".bmsg")
            .tempfile()?;
        tokio::fs::write(tmp.path(), bmessage.as_bytes()).await?;
        let path = tmp
            .path()
            .to_str()
            .ok_or_else(|| BtsmsError::Parse("temp path is not UTF-8".into()))?
            .to_string();

        go_to(&access, "telecom/msg").await?;

        // obexd defaults to UTF-8, which is what create_bmessage produces.
        let args: DictArg<'_> = HashMap::new();

        tracing::info!(
            recipient,
            body = %crate::logging::body_summary(text),
            "pushing SMS to phone outbox"
        );
        let watcher = session.watch_transfers().await?;
        let (transfer, _props) = access
            .push_message(&path, "outbox", args)
            .await
            .map_err(map_error)?;
        watcher.wait(&transfer, None, PUSH_TIMEOUT).await?;
        tracing::info!(recipient, "SMS accepted by phone");
        Ok(())
    }

    /// Mark a message as read on the phone.
    pub async fn mark_as_read(&self, message: &MapMessage) -> Result<()> {
        let _guard = self.op_lock.lock().await;
        let session = self.session()?;
        let proxy = MessageProxy::builder(session.connection())
            .path(OwnedObjectPath::try_from(message.path.as_str())?)
            .build()
            .await?;
        proxy.set_read(true).await.map_err(map_error)?;
        Ok(())
    }
}

/// Deletes a file when dropped (obexd's temporary download files).
struct RemoveOnDrop(std::path::PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Navigate to an absolute folder below the MAP root.
async fn go_to(access: &MessageAccessProxy<'_>, folder: &str) -> Result<()> {
    access.set_folder("/").await.map_err(map_error)?;
    access.set_folder(folder).await.map_err(|e| {
        tracing::debug!(folder, error = %e, "SetFolder failed");
        map_error(e)
    })
}

/// After CreateSession the MAP interface may take a moment to appear (or the phone may
/// still be asking the user). Poll a harmless SetFolder until it works.
async fn wait_for_session_ready(session: &ObexSession) -> Result<()> {
    let deadline = tokio::time::Instant::now() + SESSION_READY_TIMEOUT;
    let access = MessageAccessProxy::builder(session.connection())
        .path(session.path().clone())
        .build()
        .await?;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match access.set_folder("/").await {
            Ok(()) => {
                tracing::debug!(attempts, "MAP session ready");
                return Ok(());
            }
            Err(e) if is_unknown_object(&e) && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Err(e) => {
                tracing::warn!(attempts, error = %e, "MAP session did not become ready");
                return Err(if is_unknown_object(&e) {
                    BtsmsError::Bluetooth(
                        "The phone did not open the message access session. iPhone: enable \
                         'Show Notifications' for this computer in Settings > Bluetooth and \
                         keep the phone unlocked."
                            .into(),
                    )
                } else {
                    map_error(e)
                });
            }
        }
    }
}

/// Make MAP errors readable.
fn map_error(err: zbus::Error) -> BtsmsError {
    let name = dbus_error_name(&err).unwrap_or_default();
    if name.ends_with("Forbidden") || err.to_string().contains("Permission denied") {
        return BtsmsError::Bluetooth(
            "The phone denied message access. iPhone: enable 'Show Notifications' for this \
             computer in Settings > Bluetooth. Android: allow message access."
                .into(),
        );
    }
    if is_unknown_object(&err) {
        return BtsmsError::Bluetooth(
            "The MAP session is gone (phone disconnected or out of range)".into(),
        );
    }
    BtsmsError::DBus(err)
}

/// Turn one `ListMessages` entry into a [`MapMessage`].
pub fn parse_listing_entry(
    path: &OwnedObjectPath,
    folder: MapFolder,
    props: &PropMap,
) -> Option<MapMessage> {
    let handle = handle_from_path(path.as_str())?;
    let non_empty = |s: Option<String>| s.filter(|v| !v.trim().is_empty());
    Some(MapMessage {
        path: path.as_str().to_string(),
        handle,
        folder,
        subject: prop_string(props, "Subject").unwrap_or_default(),
        timestamp: prop_string(props, "Timestamp").unwrap_or_default(),
        sender: prop_string(props, "Sender").unwrap_or_default(),
        sender_address: non_empty(prop_string(props, "SenderAddress")),
        recipient: non_empty(prop_string(props, "Recipient")),
        recipient_address: non_empty(prop_string(props, "RecipientAddress")),
        message_type: prop_string(props, "Type").unwrap_or_else(|| "sms-gsm".into()),
        size: prop_u64(props, "Size").unwrap_or(0),
        read: prop_bool(props, "Read").unwrap_or(false),
    })
}

/// `/org/bluez/obex/client/session0/message123` -> `123`
pub fn handle_from_path(path: &str) -> Option<String> {
    path.rsplit('/')
        .next()
        .and_then(|s| s.strip_prefix("message"))
        .filter(|h| !h.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::OwnedVariant;

    fn ov(v: Variant<'_>) -> OwnedVariant {
        v.try_into_owned().unwrap()
    }

    #[test]
    fn handles_are_extracted_from_paths() {
        assert_eq!(
            handle_from_path("/org/bluez/obex/client/session2/message760932394975998988")
                .as_deref(),
            Some("760932394975998988")
        );
        assert_eq!(handle_from_path("/org/bluez/obex/client/session2"), None);
        assert_eq!(handle_from_path("/a/message"), None);
        assert_eq!(handle_from_path(""), None);
    }

    #[test]
    fn listing_entry_like_ios() {
        let mut props = PropMap::new();
        props.insert("Subject".into(), ov(Variant::from("Hello there")));
        props.insert("Timestamp".into(), ov(Variant::from("20261120T124531")));
        props.insert("Sender".into(), ov(Variant::from("Jan")));
        props.insert("SenderAddress".into(), ov(Variant::from("+48512345678")));
        props.insert("Recipient".into(), ov(Variant::from("")));
        props.insert("RecipientAddress".into(), ov(Variant::from("")));
        props.insert("Type".into(), ov(Variant::from("sms-gsm")));
        props.insert("Size".into(), ov(Variant::from(11u64)));
        props.insert("Read".into(), ov(Variant::from(true)));
        let path = OwnedObjectPath::try_from("/org/bluez/obex/client/session1/message42").unwrap();

        let msg = parse_listing_entry(&path, MapFolder::Inbox, &props).unwrap();
        assert_eq!(msg.handle, "42");
        assert_eq!(msg.sender_address.as_deref(), Some("+48512345678"));
        assert_eq!(msg.recipient_address, None, "empty strings become None");
        assert_eq!(msg.size, 11);
        assert!(msg.read);
        assert!(msg.subject_is_complete());
    }

    #[test]
    fn truncated_subject_needs_download() {
        let msg = MapMessage {
            path: String::new(),
            handle: "1".into(),
            folder: MapFolder::Inbox,
            subject: "Zażółć".into(), // 6 chars, 10 bytes
            timestamp: String::new(),
            sender: String::new(),
            sender_address: None,
            recipient: None,
            recipient_address: None,
            message_type: "sms-gsm".into(),
            size: 300,
            read: false,
        };
        assert!(!msg.subject_is_complete());
        // Size in UTF-16 units (Android): 6 -> complete
        let complete = MapMessage {
            size: 6,
            ..msg.clone()
        };
        assert!(complete.subject_is_complete());
        // Size in bytes (10): treated as incomplete. Costs one extra download on phones that
        // count bytes, but never stores a cut-off message.
        let bytes = MapMessage {
            size: 10,
            ..msg.clone()
        };
        assert!(!bytes.subject_is_complete());
        let unknown = MapMessage {
            size: 0,
            ..msg.clone()
        };
        assert!(!unknown.subject_is_complete());

        // Android: Size in UTF-16 units. A 400-char Cyrillic text cut at 256 chars has more
        // bytes than 400 but must still be downloaded.
        let cut: String = "ж".repeat(256);
        let android = MapMessage {
            subject: cut,
            size: 400,
            ..msg.clone()
        };
        assert!(!android.subject_is_complete());

        // A text at the subject limit is never trusted, even if sizes happen to match.
        let at_limit = MapMessage {
            subject: "a".repeat(256),
            size: 256,
            ..msg.clone()
        };
        assert!(!at_limit.subject_is_complete());

        // Emoji: 2 UTF-16 units each
        let emoji = MapMessage {
            subject: "👋👋".into(),
            size: 4,
            ..msg
        };
        assert!(emoji.subject_is_complete());
    }

    #[test]
    fn folder_names() {
        assert_eq!(MapFolder::Inbox.to_string(), "inbox");
        assert_eq!(MapFolder::Sent.as_str(), "sent");
    }

    #[test]
    fn unconnected_client_reports_not_connected() {
        let client = MapClient::new("AA:BB:CC:DD:EE:FF".into());
        assert!(!client.is_connected());
        assert!(matches!(client.session(), Err(BtsmsError::NotConnected)));
    }
}
