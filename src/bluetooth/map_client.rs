use crate::bluetooth::{dbus_proxies::*, vmessage};
use crate::error::{BtsmsError, Result};
use std::collections::HashMap;
use zbus::zvariant::{ObjectPath, Value};
use zbus::Connection;

/// MAP (Message Access Profile) client for SMS operations
pub struct MapClient {
    session_path: Option<String>,
    device_address: String,
    /// Keep D-Bus connection alive to prevent session termination
    _connection: Option<Connection>,
}

#[derive(Debug, Clone)]
pub struct MapMessage {
    pub handle: String,
    pub subject: String,
    pub timestamp: String,
    pub sender: String,
    pub recipient: Option<String>,
    pub message_type: String,
    pub size: u64,
    pub read: bool,
}

impl MapClient {
    /// Create a new MAP client
    pub fn new(device_address: String) -> Self {
        Self {
            session_path: None,
            device_address,
            _connection: None,
        }
    }

    /// Connect to MAP session
    pub async fn connect(&mut self) -> Result<()> {
        // Create and store the D-Bus connection to keep it alive
        let connection = Connection::session().await?;

        let client = ObexClientProxy::new(&connection).await?;

        // Create MAP session with Message Access Profile UUID
        let mut args: HashMap<&str, Value> = HashMap::new();
        args.insert("Target", Value::new("map"));

        let session_path = client
            .create_session(&self.device_address, args)
            .await
            .map_err(|e| {
                if e.to_string().contains("Unable to find service record") {
                    BtsmsError::Bluetooth(
                        "MAP service not available. For iOS: ensure phone is unlocked and \
                         paired device has notification access enabled in Settings > Bluetooth > [Device]."
                            .to_string(),
                    )
                } else {
                    BtsmsError::DBus(e)
                }
            })?;

        let path_str = session_path.to_string();

        // Store the connection to keep the session alive
        self._connection = Some(connection);

        // Wait for the MAP session to be fully established
        self.wait_for_session_ready(&path_str).await?;

        self.session_path = Some(path_str);

        Ok(())
    }

    /// Wait for the MAP session to be fully established
    async fn wait_for_session_ready(&self, session_path: &str) -> Result<()> {
        // Try to access the session for up to 15 seconds (iOS can be slow)
        for attempt in 0..30 {
            match connect_map(session_path.to_string()).await {
                Ok(proxy) => {
                    // Try to call SetFolder to verify the interface is actually available
                    match proxy.set_folder("telecom/msg").await {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            let err_str = e.to_string();
                            // UnknownMethod/UnknownObject means session not ready yet
                            if (err_str.contains("UnknownMethod")
                                || err_str.contains("UnknownObject"))
                                && attempt < 29
                            {
                                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                continue;
                            }
                            // Forbidden/Permission denied - user denied access
                            if err_str.contains("Forbidden") || err_str.contains("Permission denied")
                            {
                                return Err(BtsmsError::Bluetooth(
                                    "MAP access denied by phone. For iOS: enable notification access in \
                                     Settings > Bluetooth > [Device] and ensure phone is unlocked."
                                        .to_string(),
                                ));
                            }
                            return Err(BtsmsError::DBus(e));
                        }
                    }
                }
                Err(e) => {
                    if attempt < 29 {
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(BtsmsError::Bluetooth(
            "MAP session failed to establish. The phone may have rejected the connection. \
             For iOS: ensure notification access is enabled and phone is unlocked."
                .to_string(),
        ))
    }

    /// Disconnect from MAP session
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(session_path) = self.session_path.take() {
            // Use the stored connection to remove the session
            if let Some(connection) = &self._connection {
                let client = ObexClientProxy::new(connection).await?;
                let path = ObjectPath::try_from(session_path.as_str())
                    .map_err(|e| BtsmsError::Parse(format!("Invalid session path: {}", e)))?;
                // Ignore errors during disconnect - the session might already be closed
                let _ = client.remove_session(path).await;
            }
            self._connection = None;
        }
        Ok(())
    }

    /// List messages in inbox
    pub async fn list_inbox_messages(&self) -> Result<Vec<MapMessage>> {
        self.list_messages_in_folder("inbox").await
    }

    /// List sent messages
    pub async fn list_sent_messages(&self) -> Result<Vec<MapMessage>> {
        self.list_messages_in_folder("sent").await
    }

    /// List messages in a specific folder
    async fn list_messages_in_folder(&self, folder: &str) -> Result<Vec<MapMessage>> {
        let session_path = self
            .session_path
            .as_ref()
            .ok_or(BtsmsError::NotConnected)?;

        eprintln!("[DEBUG] list_messages_in_folder: session_path={}", session_path);
        let map_proxy = connect_map(session_path.clone()).await?;

        // We're already at telecom/msg from the connection setup, so just navigate to subfolder
        eprintln!("[DEBUG] Setting folder to: {}", folder);
        map_proxy
            .set_folder(folder)
            .await
            .map_err(|e| {
                eprintln!("[DEBUG] set_folder('{}') error: {:?}", folder, e);
                BtsmsError::DBus(e)
            })?;

        // List messages with empty filter (get all)
        eprintln!("[DEBUG] Calling list_messages...");
        let filter: HashMap<&str, Value> = HashMap::new();
        let messages = map_proxy
            .list_messages("", filter)
            .await
            .map_err(|e| {
                eprintln!("[DEBUG] list_messages error: {:?}", e);
                BtsmsError::DBus(e)
            })?;

        // Parse messages - the response is a dict mapping object paths to properties
        // The handle is extracted from the object path (e.g., /org/bluez/obex/client/session0/message123 -> "123")
        let mut result = Vec::new();
        for (object_path, msg_data) in messages {
            let path_str = object_path.as_str();
            // Extract handle from object path - it's the part after "message"
            let handle = path_str
                .rsplit('/')
                .next()
                .and_then(|s| s.strip_prefix("message"))
                .map(|s| s.to_string())
                .unwrap_or_default();

            eprintln!("[DEBUG] Message object_path: {}, handle: {}", object_path, handle);
            if let Some(msg) = Self::parse_message_metadata_with_handle(handle, msg_data) {
                result.push(msg);
            }
        }

        Ok(result)
    }

    /// Get full message content by handle
    pub async fn get_message_content(&self, handle: &str) -> Result<String> {
        let session_path = self
            .session_path
            .as_ref()
            .ok_or(BtsmsError::NotConnected)?;

        let map_proxy = connect_map(session_path.clone()).await?;

        // Create temporary file for message content
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("btsms_msg_{}.txt", handle));
        let temp_path = temp_file
            .to_str()
            .ok_or(BtsmsError::Parse("Invalid temp path".to_string()))?;

        // Get message and save to file
        let (transfer_path, _properties) = map_proxy
            .get_message(handle, temp_path, false)
            .await
            .map_err(BtsmsError::DBus)?;

        // Wait for transfer to complete
        self.wait_for_transfer(&transfer_path.to_string()).await?;

        // Read message content from file
        let content = tokio::fs::read_to_string(&temp_file)
            .await
            .map_err(|e| BtsmsError::Parse(format!("Failed to read message file: {}", e)))?;

        // Parse vMessage format to extract body
        let parsed = vmessage::parse_vmessage(&content)?;
        let body = parsed.body;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_file).await;

        Ok(body)
    }

    /// Send SMS message
    pub async fn send_sms(&self, recipient: &str, message: &str) -> Result<()> {
        let session_path = self
            .session_path
            .as_ref()
            .ok_or(BtsmsError::NotConnected)?;

        let map_proxy = connect_map(session_path.clone()).await?;

        // Normalize phone number
        let normalized_recipient = crate::contacts::normalize_e164(recipient)?;

        // Create vMessage/BMSG format
        let vmessage_content = vmessage::create_vmessage(&normalized_recipient, "Self", message);

        // Write vMessage to temporary file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("btsms_send_{}.txt", chrono::Utc::now().timestamp()));
        let temp_path = temp_file
            .to_str()
            .ok_or(BtsmsError::Parse("Invalid temp path".to_string()))?;

        tokio::fs::write(&temp_file, &vmessage_content)
            .await
            .map_err(|e| BtsmsError::Parse(format!("Failed to write message file: {}", e)))?;

        eprintln!("[DEBUG] send_sms: navigating to outbox folder");
        // Navigate to outbox folder (relative - we're already at telecom/msg from connection setup)
        map_proxy
            .set_folder("outbox")
            .await
            .map_err(|e| {
                eprintln!("[DEBUG] set_folder('outbox') error: {:?}", e);
                BtsmsError::DBus(e)
            })?;

        eprintln!("[DEBUG] send_sms: pushing message from file: {}", temp_path);
        eprintln!("[DEBUG] vMessage content:\n{}", vmessage_content);

        // Push message via MAP (empty folder = current folder which is outbox)
        let args: HashMap<&str, Value> = HashMap::new();

        let (transfer_path, _properties) = map_proxy
            .push_message(temp_path, "", args)
            .await
            .map_err(BtsmsError::DBus)?;

        // Wait for transfer to complete
        self.wait_for_transfer(&transfer_path.to_string()).await?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_file).await;

        Ok(())
    }

    /// Mark message as read
    pub async fn mark_as_read(&self, handle: &str) -> Result<()> {
        let session_path = self
            .session_path
            .as_ref()
            .ok_or(BtsmsError::NotConnected)?;

        let map_proxy = connect_map(session_path.clone()).await?;

        // Create temp file for status update
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("btsms_status_{}.txt", handle));
        let temp_path = temp_file
            .to_str()
            .ok_or(BtsmsError::Parse("Invalid temp path".to_string()))?;

        // Update inbox with read status
        map_proxy
            .update_inbox(temp_path)
            .await
            .map_err(BtsmsError::DBus)?;

        Ok(())
    }

    /// Wait for OBEX transfer to complete
    async fn wait_for_transfer(&self, transfer_path: &str) -> Result<()> {
        let transfer_proxy = connect_transfer(transfer_path.to_string()).await?;

        // Poll transfer status
        for _ in 0..100 {
            // Max 10 seconds (100 * 100ms)
            match transfer_proxy.status().await {
                Ok(status) => {
                    match status.as_str() {
                        "complete" => return Ok(()),
                        "error" => return Err(BtsmsError::Bluetooth("Transfer failed".to_string())),
                        _ => {
                            // Still in progress
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        }
                    }
                }
                Err(e) => {
                    return Err(BtsmsError::DBus(e));
                }
            }
        }

        Err(BtsmsError::Bluetooth("Transfer timeout".to_string()))
    }

    /// Parse message metadata from D-Bus response with handle extracted from object path
    fn parse_message_metadata_with_handle(
        handle: String,
        data: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> Option<MapMessage> {
        if handle.is_empty() {
            return None;
        }

        let subject = data
            .get("Subject")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .unwrap_or_default();

        let timestamp = data
            .get("Timestamp")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .unwrap_or_default();

        let sender = data
            .get("Sender")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .unwrap_or_default();

        let recipient = data
            .get("Recipient")
            .and_then(|v| v.downcast_ref::<String>().ok());

        let message_type = data
            .get("Type")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .unwrap_or_else(|| "SMS".to_string());

        let size = data
            .get("Size")
            .and_then(|v| v.downcast_ref::<u64>().ok())
            .unwrap_or(0);

        let read = data
            .get("Read")
            .and_then(|v| v.downcast_ref::<bool>().ok())
            .unwrap_or(false);

        Some(MapMessage {
            handle,
            subject,
            timestamp,
            sender,
            recipient,
            message_type,
            size,
            read,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_map_client_creation() {
        let client = MapClient::new("AA:BB:CC:DD:EE:FF".to_string());
        assert_eq!(client.device_address, "AA:BB:CC:DD:EE:FF");
        assert!(client.session_path.is_none());
    }

    #[tokio::test]
    async fn test_map_connect_without_device() {
        let mut client = MapClient::new("00:00:00:00:00:00".to_string());
        // This will fail without a real device, but tests the code path
        match client.connect().await {
            Ok(_) => {
                // Clean up
                let _ = client.disconnect().await;
            }
            Err(_) => {
                // Expected when no device available
            }
        }
    }
}
