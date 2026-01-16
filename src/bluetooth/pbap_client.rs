use crate::bluetooth::dbus_proxies::*;
use crate::error::{BtsmsError, Result};
use std::collections::HashMap;
use zbus::zvariant::{ObjectPath, Value};

/// PBAP (Phonebook Access Profile) client for contact synchronization
pub struct PbapClient {
    session_path: Option<String>,
    device_address: String,
}

impl PbapClient {
    /// Create a new PBAP client
    pub fn new(device_address: String) -> Self {
        Self {
            session_path: None,
            device_address,
        }
    }

    /// Connect to PBAP session
    pub async fn connect(&mut self) -> Result<()> {
        let client = connect_obex().await?;

        // Create PBAP session with Phonebook Access Profile UUID
        let mut args: HashMap<&str, Value> = HashMap::new();
        args.insert("Target", Value::new("pbap"));

        let session_path = client
            .create_session(&self.device_address, args)
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        self.session_path = Some(session_path.to_string());

        Ok(())
    }

    /// Disconnect from PBAP session
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

    /// List all contacts (returns vCard handles)
    pub async fn list_contacts(&self) -> Result<Vec<(String, String)>> {
        let session_path = self
            .session_path
            .as_ref()
            .ok_or(BtsmsError::NotConnected)?;

        let pbap_proxy = connect_pbap(session_path).await?;

        // Select internal phonebook
        pbap_proxy
            .select("int", "pb")
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        // List all vCards
        let filter: HashMap<&str, Value> = HashMap::new();
        let contacts = pbap_proxy
            .list(filter)
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        Ok(contacts)
    }

    /// Pull all contacts as vCard stream
    pub async fn pull_all_contacts(&self) -> Result<String> {
        let session_path = self
            .session_path
            .as_ref()
            .ok_or(BtsmsError::NotConnected)?;

        let pbap_proxy = connect_pbap(session_path).await?;

        // Select internal phonebook
        pbap_proxy
            .select("int", "pb")
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        // Create temporary file for vCard data
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("btsms_contacts_{}.vcf", chrono::Utc::now().timestamp()));
        let temp_path = temp_file
            .to_str()
            .ok_or(BtsmsError::Parse("Invalid temp path".to_string()))?;

        // Pull all contacts
        let mut filter: HashMap<&str, Value> = HashMap::new();
        filter.insert("Format", Value::new("vcard30")); // vCard 3.0 format

        let (transfer_path, _properties) = pbap_proxy
            .pull_all(temp_path, filter)
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        // Wait for transfer to complete
        self.wait_for_transfer(&transfer_path.to_string()).await?;

        // Read vCard data from file
        let vcards = tokio::fs::read_to_string(&temp_file)
            .await
            .map_err(|e| BtsmsError::Parse(format!("Failed to read vCard file: {}", e)))?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_file).await;

        Ok(vcards)
    }

    /// Pull single contact by handle
    pub async fn pull_contact(&self, handle: &str) -> Result<String> {
        let session_path = self
            .session_path
            .as_ref()
            .ok_or(BtsmsError::NotConnected)?;

        let pbap_proxy = connect_pbap(session_path).await?;

        // Create temporary file for vCard data
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("btsms_contact_{}.vcf", handle));
        let temp_path = temp_file
            .to_str()
            .ok_or(BtsmsError::Parse("Invalid temp path".to_string()))?;

        // Pull single vCard
        let mut filter: HashMap<&str, Value> = HashMap::new();
        filter.insert("Format", Value::new("vcard30"));

        let (transfer_path, _properties) = pbap_proxy
            .pull(handle, temp_path, filter)
            .await
            .map_err(|e| BtsmsError::DBus(e))?;

        // Wait for transfer to complete
        self.wait_for_transfer(&transfer_path.to_string()).await?;

        // Read vCard data
        let vcard = tokio::fs::read_to_string(&temp_file)
            .await
            .map_err(|e| BtsmsError::Parse(format!("Failed to read vCard file: {}", e)))?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_file).await;

        Ok(vcard)
    }

    /// Parse vCard stream into individual vCards
    pub fn parse_vcard_stream(vcards: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current_vcard = String::new();
        let mut in_vcard = false;

        for line in vcards.lines() {
            if line.starts_with("BEGIN:VCARD") {
                in_vcard = true;
                current_vcard.clear();
                current_vcard.push_str(line);
                current_vcard.push('\n');
            } else if line.starts_with("END:VCARD") {
                current_vcard.push_str(line);
                current_vcard.push('\n');
                result.push(current_vcard.clone());
                in_vcard = false;
            } else if in_vcard {
                current_vcard.push_str(line);
                current_vcard.push('\n');
            }
        }

        result
    }

    /// Wait for OBEX transfer to complete
    async fn wait_for_transfer(&self, transfer_path: &str) -> Result<()> {
        let transfer_proxy = connect_transfer(transfer_path).await?;

        // Poll transfer status
        for _ in 0..100 {
            // Max 10 seconds
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vcard_stream() {
        let vcards = r#"BEGIN:VCARD
VERSION:3.0
FN:John Doe
TEL:+15551234567
END:VCARD
BEGIN:VCARD
VERSION:3.0
FN:Jane Smith
TEL:+15559876543
END:VCARD"#;

        let parsed = PbapClient::parse_vcard_stream(vcards);
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].contains("John Doe"));
        assert!(parsed[1].contains("Jane Smith"));
    }

    #[tokio::test]
    async fn test_pbap_client_creation() {
        let client = PbapClient::new("AA:BB:CC:DD:EE:FF".to_string());
        assert_eq!(client.device_address, "AA:BB:CC:DD:EE:FF");
        assert!(client.session_path.is_none());
    }

    #[tokio::test]
    async fn test_pbap_connect_without_device() {
        let mut client = PbapClient::new("00:00:00:00:00:00".to_string());
        // This will fail without a real device
        match client.connect().await {
            Ok(_) => {
                let _ = client.disconnect().await;
            }
            Err(_) => {
                // Expected when no device available
            }
        }
    }
}
