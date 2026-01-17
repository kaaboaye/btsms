use crate::bluetooth::dbus_proxies::*;
use crate::error::{BtsmsError, Result};
use std::collections::HashMap;
use zbus::zvariant::{ObjectPath, Value};

/// Convert PBAP D-Bus errors to user-friendly error messages
fn map_pbap_error(e: zbus::Error) -> BtsmsError {
    let err_str = e.to_string();
    if err_str.contains("doesn't exist") || err_str.contains("UnknownObject") {
        BtsmsError::Bluetooth(
            "PBAP access was rejected by the phone. \
             For Android: check your phone for a 'Contact Sharing' permission request, \
             or enable it in Settings > Connected devices > [Device] > Contact Sharing. \
             For iOS: enable 'Show Notifications' in Settings > Bluetooth > [Device] > (i)."
                .to_string(),
        )
    } else if err_str.contains("Forbidden") || err_str.contains("Permission denied") {
        BtsmsError::Bluetooth(
            "PBAP access denied by phone. \
             For iOS: enable 'Show Notifications' in Settings > Bluetooth > [Device] > (i). \
             For Android: enable 'Contact Sharing' in Bluetooth settings for this device."
                .to_string(),
        )
    } else {
        BtsmsError::DBus(e)
    }
}

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
    ///
    /// Note: iOS requires the user to enable "Show Notifications" in
    /// Settings > Bluetooth > [Device Name] > (i) for PBAP access.
    pub async fn connect(&mut self) -> Result<()> {
        let client = connect_obex().await?;

        // Create PBAP session with Phonebook Access Profile UUID
        let mut args: HashMap<&str, Value> = HashMap::new();
        args.insert("Target", Value::new("pbap"));

        let session_path = client
            .create_session(&self.device_address, args)
            .await
            .map_err(|e| {
                if e.to_string().contains("Unable to find service record") {
                    BtsmsError::Bluetooth(
                        "PBAP service not available. For iOS: ensure 'Show Notifications' is enabled in \
                         Settings > Bluetooth > [Device] > (i), and the phone is unlocked.".to_string()
                    )
                } else {
                    BtsmsError::DBus(e)
                }
            })?;

        let path_str = session_path.to_string();

        // Verify the session is actually established by waiting for the
        // PhonebookAccess1 interface to become available
        self.wait_for_session_ready(&path_str).await?;

        self.session_path = Some(path_str);

        Ok(())
    }

    /// Wait for the PBAP session to be fully established
    ///
    /// iOS devices may take time to establish the session, especially if
    /// the user needs to authorize the connection on the phone.
    async fn wait_for_session_ready(&self, session_path: &str) -> Result<()> {
        // Try to access the session for up to 10 seconds
        for attempt in 0..20 {
            match connect_pbap(session_path.to_string()).await {
                Ok(proxy) => {
                    // Try to call select to verify the interface is actually available
                    match proxy.select("int", "pb").await {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            let err_str = e.to_string();
                            // Permission denied - session exists but access denied
                            if err_str.contains("Forbidden")
                                || err_str.contains("Permission denied")
                            {
                                return Err(BtsmsError::Bluetooth(
                                    "PBAP access denied by phone. \
                                     For iOS: enable 'Show Notifications' in Settings > Bluetooth > [Device] > (i). \
                                     For Android: enable 'Contact Sharing' in Bluetooth settings for this device.".to_string()
                                ));
                            }
                            // UnknownObject means session not ready yet or phone disconnected
                            if err_str.contains("UnknownObject") && attempt < 19 {
                                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                continue;
                            }
                            // Check for method not existing - phone rejected contact access
                            if err_str.contains("doesn't exist") {
                                return Err(BtsmsError::Bluetooth(
                                    "PBAP session established but access was rejected. \
                                     For Android: check your phone for a 'Contact Sharing' permission request, \
                                     or enable it in Settings > Connected devices > [Device] > Contact Sharing. \
                                     For iOS: enable 'Show Notifications' in Settings > Bluetooth > [Device] > (i).".to_string()
                                ));
                            }
                            return Err(BtsmsError::DBus(e));
                        }
                    }
                }
                Err(e) => {
                    if attempt < 19 {
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(BtsmsError::Bluetooth(
            "PBAP session failed to establish. The phone may have rejected the connection. \
             For Android: enable 'Contact Sharing' in Bluetooth settings for this device. \
             For iOS: ensure 'Show Notifications' is enabled and phone is unlocked."
                .to_string(),
        ))
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
                .map_err(BtsmsError::DBus)?;
            self.session_path = None;
        }
        Ok(())
    }

    /// List all contacts (returns vCard handles)
    pub async fn list_contacts(&self) -> Result<Vec<(String, String)>> {
        let session_path = self.session_path.as_ref().ok_or(BtsmsError::NotConnected)?;

        let pbap_proxy = connect_pbap(session_path.clone()).await?;

        // Select internal phonebook
        pbap_proxy
            .select("int", "pb")
            .await
            .map_err(map_pbap_error)?;

        // List all vCards
        let filter: HashMap<&str, Value> = HashMap::new();
        let contacts = pbap_proxy.list(filter).await.map_err(map_pbap_error)?;

        Ok(contacts)
    }

    /// Pull all contacts as vCard stream
    pub async fn pull_all_contacts(&self) -> Result<String> {
        let session_path = self.session_path.as_ref().ok_or(BtsmsError::NotConnected)?;

        let pbap_proxy = connect_pbap(session_path.clone()).await?;

        // Select internal phonebook
        pbap_proxy
            .select("int", "pb")
            .await
            .map_err(map_pbap_error)?;

        // Create temporary file for vCard data
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!(
            "btsms_contacts_{}.vcf",
            chrono::Utc::now().timestamp()
        ));
        let temp_path = temp_file
            .to_str()
            .ok_or(BtsmsError::Parse("Invalid temp path".to_string()))?;

        // Pull all contacts
        let mut filter: HashMap<&str, Value> = HashMap::new();
        filter.insert("Format", Value::new("vcard30")); // vCard 3.0 format

        let (transfer_path, _properties) = pbap_proxy
            .pull_all(temp_path, filter)
            .await
            .map_err(map_pbap_error)?;

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
        let session_path = self.session_path.as_ref().ok_or(BtsmsError::NotConnected)?;

        let pbap_proxy = connect_pbap(session_path.clone()).await?;

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
            .map_err(map_pbap_error)?;

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
        let transfer_proxy = connect_transfer(transfer_path.to_string()).await?;

        // Poll transfer status
        for _ in 0..100 {
            // Max 10 seconds
            match transfer_proxy.status().await {
                Ok(status) => {
                    match status.as_str() {
                        "complete" => return Ok(()),
                        "error" => {
                            return Err(BtsmsError::Bluetooth("Transfer failed".to_string()))
                        }
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
