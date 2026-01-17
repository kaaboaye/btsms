use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Manager, Peripheral};
use crate::error::{BtsmsError, Result};
use futures::stream::StreamExt;
use std::collections::HashMap;
use tokio::sync::mpsc;
use uuid::Uuid;

/// ANCS (Apple Notification Center Service) UUIDs
const ANCS_SERVICE_UUID: Uuid = Uuid::from_u128(0x7905F431_B5CE_4E99_A40F_4B1E122D00D0);
const ANCS_NOTIFICATION_SOURCE: Uuid = Uuid::from_u128(0x9FBF120D_6301_42F9_8265_CCB7C68E4E28);
const ANCS_CONTROL_POINT: Uuid = Uuid::from_u128(0x69D1D8F3_45E1_49A8_9821_9BBDFDAAD9D9);
const ANCS_DATA_SOURCE: Uuid = Uuid::from_u128(0x22EAC6E9_2460_4A6C_BEE1_38A40CDFD0C4);

/// Notification category IDs
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum NotificationCategory {
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

impl From<u8> for NotificationCategory {
    fn from(val: u8) -> Self {
        match val {
            1 => Self::IncomingCall,
            2 => Self::MissedCall,
            3 => Self::Voicemail,
            4 => Self::Social,
            5 => Self::Schedule,
            6 => Self::Email,
            7 => Self::News,
            8 => Self::HealthAndFitness,
            9 => Self::BusinessAndFinance,
            10 => Self::Location,
            11 => Self::Entertainment,
            _ => Self::Other,
        }
    }
}

/// ANCS Notification
#[derive(Debug, Clone)]
pub struct AncsNotification {
    pub uid: u32,
    pub category: NotificationCategory,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub message: Option<String>,
    pub app_identifier: Option<String>,
}

/// ANCS Client for receiving iPhone notifications
pub struct AncsClient {
    peripheral: Option<Peripheral>,
    notification_rx: Option<mpsc::UnboundedReceiver<AncsNotification>>,
}

impl Default for AncsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AncsClient {
    pub fn new() -> Self {
        Self {
            peripheral: None,
            notification_rx: None,
        }
    }

    /// Discover and connect to iPhone with ANCS
    pub async fn connect(&mut self) -> Result<()> {
        let manager = Manager::new()
            .await
            .map_err(|e| BtsmsError::Bluetooth(format!("Failed to create BLE manager: {}", e)))?;

        let adapters = manager.adapters().await.map_err(|e| {
            BtsmsError::Bluetooth(format!("Failed to get BLE adapters: {}", e))
        })?;

        let adapter = adapters
            .into_iter()
            .next()
            .ok_or(BtsmsError::Bluetooth("No BLE adapter found".to_string()))?;

        // Start scanning
        adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| BtsmsError::Bluetooth(format!("Failed to start scan: {}", e)))?;

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        // Find iPhone with ANCS service
        let peripherals = adapter.peripherals().await.map_err(|e| {
            BtsmsError::Bluetooth(format!("Failed to get peripherals: {}", e))
        })?;

        for peripheral in peripherals {
            let properties = peripheral.properties().await.map_err(|e| {
                BtsmsError::Bluetooth(format!("Failed to get properties: {}", e))
            })?;

            if let Some(props) = properties {
                let services = props.services;
                if services.contains(&ANCS_SERVICE_UUID) {
                    self.peripheral = Some(peripheral.clone());

                    // Connect to the peripheral
                    peripheral.connect().await.map_err(|e| {
                        BtsmsError::Bluetooth(format!("Failed to connect: {}", e))
                    })?;

                    // Discover services
                    peripheral.discover_services().await.map_err(|e| {
                        BtsmsError::Bluetooth(format!("Failed to discover services: {}", e))
                    })?;

                    // Setup notification handler
                    self.setup_notifications(peripheral).await?;

                    return Ok(());
                }
            }
        }

        Err(BtsmsError::Bluetooth("No iPhone with ANCS found".to_string()))
    }

    /// Setup ANCS notification handling
    async fn setup_notifications(&mut self, peripheral: Peripheral) -> Result<()> {
        // Find characteristics
        let chars = peripheral.characteristics();

        let notification_source = chars
            .iter()
            .find(|c| c.uuid == ANCS_NOTIFICATION_SOURCE)
            .ok_or(BtsmsError::Bluetooth("ANCS Notification Source not found".to_string()))?;

        let control_point = chars
            .iter()
            .find(|c| c.uuid == ANCS_CONTROL_POINT)
            .ok_or(BtsmsError::Bluetooth("ANCS Control Point not found".to_string()))?;

        let data_source = chars
            .iter()
            .find(|c| c.uuid == ANCS_DATA_SOURCE)
            .ok_or(BtsmsError::Bluetooth("ANCS Data Source not found".to_string()))?;

        // Subscribe to notifications
        peripheral
            .subscribe(notification_source)
            .await
            .map_err(|e| BtsmsError::Bluetooth(format!("Failed to subscribe to notifications: {}", e)))?;

        peripheral
            .subscribe(data_source)
            .await
            .map_err(|e| BtsmsError::Bluetooth(format!("Failed to subscribe to data source: {}", e)))?;

        // Create channel for notifications
        let (tx, rx) = mpsc::unbounded_channel();
        self.notification_rx = Some(rx);

        // Spawn task to handle notifications
        let peripheral_clone = peripheral.clone();
        let control_point_clone = control_point.clone();

        tokio::spawn(async move {
            let mut notification_stream = peripheral_clone.notifications().await.unwrap();
            let mut pending_notifications: HashMap<u32, AncsNotification> = HashMap::new();

            while let Some(data) = notification_stream.next().await {
                if data.uuid == ANCS_NOTIFICATION_SOURCE {
                    // Parse notification source event
                    if data.value.len() >= 8 {
                        let category_id = data.value[2];
                        let uid = u32::from_le_bytes([
                            data.value[4],
                            data.value[5],
                            data.value[6],
                            data.value[7],
                        ]);

                        let category = NotificationCategory::from(category_id);

                        // Only process Social (Messages) category
                        if category == NotificationCategory::Social {
                            // Create partial notification
                            let notif = AncsNotification {
                                uid,
                                category,
                                title: None,
                                subtitle: None,
                                message: None,
                                app_identifier: None,
                            };

                            pending_notifications.insert(uid, notif);

                            // Request notification attributes
                            let request = vec![
                                0x00, // CommandID: GetNotificationAttributes
                                data.value[4],
                                data.value[5],
                                data.value[6],
                                data.value[7], // UID
                                0x01, 0xFF, 0xFF, // AttributeID: Title, MaxLength: 65535
                                0x02, 0xFF, 0xFF, // AttributeID: Subtitle
                                0x03, 0xFF, 0xFF, // AttributeID: Message
                                0x00, 0xFF, 0xFF, // AttributeID: AppIdentifier
                            ];

                            let _ = peripheral_clone
                                .write(&control_point_clone, &request, WriteType::WithResponse)
                                .await;
                        }
                    }
                } else if data.uuid == ANCS_DATA_SOURCE {
                    // Parse data source response
                    if let Some(notif) = Self::parse_data_source(&data.value, &mut pending_notifications) {
                        let _ = tx.send(notif);
                    }
                }
            }
        });

        Ok(())
    }

    /// Parse ANCS Data Source response
    fn parse_data_source(
        data: &[u8],
        pending: &mut HashMap<u32, AncsNotification>,
    ) -> Option<AncsNotification> {
        if data.len() < 5 {
            return None;
        }

        let command_id = data[0];
        if command_id != 0x00 {
            // Not GetNotificationAttributes response
            return None;
        }

        let uid = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);

        let notif = pending.get_mut(&uid)?;

        // Parse attributes
        let mut offset = 5;
        while offset < data.len() {
            if offset + 3 > data.len() {
                break;
            }

            let attr_id = data[offset];
            let length = u16::from_le_bytes([data[offset + 1], data[offset + 2]]) as usize;
            offset += 3;

            if offset + length > data.len() {
                break;
            }

            let value = String::from_utf8_lossy(&data[offset..offset + length]).to_string();

            match attr_id {
                0x00 => notif.app_identifier = Some(value),
                0x01 => notif.title = Some(value),
                0x02 => notif.subtitle = Some(value),
                0x03 => notif.message = Some(value),
                _ => {}
            }

            offset += length;
        }

        // Return completed notification
        pending.remove(&uid)
    }

    /// Disconnect from iPhone
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(peripheral) = &self.peripheral {
            peripheral
                .disconnect()
                .await
                .map_err(|e| BtsmsError::Bluetooth(format!("Failed to disconnect: {}", e)))?;
        }
        self.peripheral = None;
        self.notification_rx = None;
        Ok(())
    }

    /// Get notification receiver
    pub fn take_notification_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<AncsNotification>> {
        self.notification_rx.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ancs_client_creation() {
        let client = AncsClient::new();
        assert!(client.peripheral.is_none());
    }

    #[test]
    fn test_notification_category_from_u8() {
        assert_eq!(NotificationCategory::from(4), NotificationCategory::Social);
        assert_eq!(NotificationCategory::from(1), NotificationCategory::IncomingCall);
        assert_eq!(NotificationCategory::from(99), NotificationCategory::Other);
    }
}
