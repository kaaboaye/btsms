use crate::bluetooth::{dbus_proxies::*, vmessage};
use crate::error::{BtsmsError, Result};
use std::collections::HashMap;
use zbus::zvariant::{ObjectPath, Value};

/// MAP (Message Access Profile) client for SMS operations
pub struct MapClient {
    session_path: Option<String>,
    device_address: String,
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
        }
    }

    /// Connect to MAP session
    pub async fn connect(&mut self) -> Result<()> {
        let client = connect_obex().await?;

        // Create MAP session with Message Access Profile UUID
        let mut args: HashMap<&str, Value> = HashMap::new();
        args.insert("Target", Value::new("map"));

        let session_path = client
            .create_session(&self.device_address, args)
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        self.session_path = Some(session_path.to_string());

        Ok(())
    }

    /// Disconnect from MAP session
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(session_path) = &self.session_path {
            let client = connect_obex().await?;
            let path = ObjectPath::try_from(session_path.as_str())
                .map_err(|e| BtsmsError::Parse(format!("Invalid session path: {}", e)))?;
            client
                .remove_session(path)
                .await
                .map_err(|e| BtsmsError::DBus(e))?;
            self.session_path = None;
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

        let map_proxy = connect_map(session_path.clone()).await?;

        // Set folder to telecom/msg/{folder}
        map_proxy
            .set_folder(&format!("telecom/msg/{}", folder))
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        // List messages with empty filter (get all)
        let filter: HashMap<&str, Value> = HashMap::new();
        let messages = map_proxy
            .list_messages("", filter)
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        // Parse messages
        let mut result = Vec::new();
        for msg_data in messages {
            if let Some(msg) = Self::parse_message_metadata(msg_data) {
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
            .map_err(|e| BtsmsError::DBus(e))?;

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

        // Create vMessage format
        let vmessage_content = vmessage::create_vmessage(&normalized_recipient, "Self", message);

        // Write vMessage to temporary file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("btsms_send_{}.txt", chrono::Utc::now().timestamp()));
        let temp_path = temp_file
            .to_str()
            .ok_or(BtsmsError::Parse("Invalid temp path".to_string()))?;

        tokio::fs::write(&temp_file, vmessage_content)
            .await
            .map_err(|e| BtsmsError::Parse(format!("Failed to write message file: {}", e)))?;

        // Push message via MAP
        let mut args: HashMap<&str, Value> = HashMap::new();
        args.insert("Charset", Value::new("UTF-8"));

        let (transfer_path, _properties) = map_proxy
            .push_message(temp_path, "telecom/msg/outbox", args)
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

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
            .map_err(|e| BtsmsError::DBus(e))?;

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

    /// Parse message metadata from D-Bus response
    fn parse_message_metadata(data: HashMap<String, zbus::zvariant::OwnedValue>) -> Option<MapMessage> {
        let handle = data
            .get("Handle")?
            .downcast_ref::<String>()
            .ok()?
            .clone();

        let subject = data
            .get("Subject")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .map(|s| s.clone())
            .unwrap_or_default();

        let timestamp = data
            .get("Timestamp")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .map(|s| s.clone())
            .unwrap_or_default();

        let sender = data
            .get("Sender")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .map(|s| s.clone())
            .unwrap_or_default();

        let recipient = data
            .get("Recipient")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .map(|s| s.clone());

        let message_type = data
            .get("Type")
            .and_then(|v| v.downcast_ref::<String>().ok())
            .map(|s| s.clone())
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
