use crate::error::{BtsmsError, Result};
use std::collections::HashMap;
use zbus::Connection;

/// Bluetooth device information
#[derive(Debug, Clone)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    pub trusted: bool,
}

/// Device manager for BlueZ via D-Bus
pub struct DeviceManager {
    connection: Connection,
}

impl DeviceManager {
    /// Create a new device manager
    pub async fn new() -> Result<Self> {
        let connection = Connection::system().await?;
        Ok(Self { connection })
    }

    /// Get all paired Bluetooth devices
    pub async fn get_paired_devices(&self) -> Result<Vec<BluetoothDevice>> {
        let proxy = zbus::Proxy::new(
            &self.connection,
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
        )
        .await?;

        // Get all managed objects
        let objects: HashMap<
            zbus::zvariant::OwnedObjectPath,
            HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>,
        > = proxy.call("GetManagedObjects", &()).await?;

        let mut devices = Vec::new();

        for (_path, interfaces) in objects {
            // Check if this object is a Bluetooth device
            if let Some(device_props) = interfaces.get("org.bluez.Device1") {
                // Get device properties
                let address = device_props
                    .get("Address")
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .unwrap_or_default();

                let name = device_props
                    .get("Name")
                    .or_else(|| device_props.get("Alias"))
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .unwrap_or_else(|| "Unknown Device".to_string());

                let paired = device_props
                    .get("Paired")
                    .and_then(|v| v.downcast_ref::<bool>().ok())
                    .unwrap_or(false);

                let connected = device_props
                    .get("Connected")
                    .and_then(|v| v.downcast_ref::<bool>().ok())
                    .unwrap_or(false);

                let trusted = device_props
                    .get("Trusted")
                    .and_then(|v| v.downcast_ref::<bool>().ok())
                    .unwrap_or(false);

                let icon = device_props
                    .get("Icon")
                    .and_then(|v| v.downcast_ref::<String>().ok());

                // Only include paired devices
                if paired {
                    eprintln!("Found device: {} ({}) - Icon: {:?}", name, address, icon);
                    devices.push(BluetoothDevice {
                        address,
                        name,
                        paired,
                        connected,
                        trusted,
                    });
                }
            }
        }

        Ok(devices)
    }

    /// Connect to a specific device
    pub async fn connect_device(&self, device_address: &str) -> Result<()> {
        // Find the device object path
        let proxy = zbus::Proxy::new(
            &self.connection,
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
        )
        .await?;

        let objects: HashMap<
            zbus::zvariant::OwnedObjectPath,
            HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>,
        > = proxy.call("GetManagedObjects", &()).await?;

        for (path, interfaces) in objects {
            if let Some(device_props) = interfaces.get("org.bluez.Device1") {
                let address = device_props
                    .get("Address")
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .unwrap_or_default();

                if address == device_address {
                    // Found the device, now connect to it
                    let device_proxy = zbus::Proxy::new(
                        &self.connection,
                        "org.bluez",
                        path.as_str(),
                        "org.bluez.Device1",
                    )
                    .await?;

                    // Call Connect method
                    let _: () = device_proxy.call("Connect", &()).await?;

                    return Ok(());
                }
            }
        }

        Err(BtsmsError::Bluetooth(format!(
            "Device {} not found",
            device_address
        )))
    }

    /// Disconnect from a specific device
    pub async fn disconnect_device(&self, device_address: &str) -> Result<()> {
        let proxy = zbus::Proxy::new(
            &self.connection,
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
        )
        .await?;

        let objects: HashMap<
            zbus::zvariant::OwnedObjectPath,
            HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>,
        > = proxy.call("GetManagedObjects", &()).await?;

        for (path, interfaces) in objects {
            if let Some(device_props) = interfaces.get("org.bluez.Device1") {
                let address = device_props
                    .get("Address")
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .unwrap_or_default();

                if address == device_address {
                    let device_proxy = zbus::Proxy::new(
                        &self.connection,
                        "org.bluez",
                        path.as_str(),
                        "org.bluez.Device1",
                    )
                    .await?;

                    let _: () = device_proxy.call("Disconnect", &()).await?;

                    return Ok(());
                }
            }
        }

        Ok(())
    }

    /// Get all paired smartphones (heuristic: has "Phone" or "iPhone" in name or icon)
    pub async fn get_all_paired_phones(&self) -> Result<Vec<BluetoothDevice>> {
        let proxy = zbus::Proxy::new(
            &self.connection,
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
        )
        .await?;

        let objects: HashMap<
            zbus::zvariant::OwnedObjectPath,
            HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>,
        > = proxy.call("GetManagedObjects", &()).await?;

        let mut phones = Vec::new();

        for (_path, interfaces) in objects {
            if let Some(device_props) = interfaces.get("org.bluez.Device1") {
                let paired = device_props
                    .get("Paired")
                    .and_then(|v| v.downcast_ref::<bool>().ok())
                    .unwrap_or(false);

                if !paired {
                    continue;
                }

                let icon = device_props
                    .get("Icon")
                    .and_then(|v| v.downcast_ref::<String>().ok());

                let name = device_props
                    .get("Name")
                    .or_else(|| device_props.get("Alias"))
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .unwrap_or_else(|| "Unknown Device".to_string());

                let address = device_props
                    .get("Address")
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .unwrap_or_default();

                let connected = device_props
                    .get("Connected")
                    .and_then(|v| v.downcast_ref::<bool>().ok())
                    .unwrap_or(false);

                let trusted = device_props
                    .get("Trusted")
                    .and_then(|v| v.downcast_ref::<bool>().ok())
                    .unwrap_or(false);

                // Check if it's a phone by icon first (most reliable)
                let is_phone = if let Some(icon_str) = &icon {
                    icon_str == "phone"
                } else {
                    // Check by name
                    let name_lower = name.to_lowercase();
                    name_lower.contains("phone")
                        || name_lower.contains("iphone")
                        || name_lower.contains("android")
                        || name_lower.contains("pixel")
                        || name_lower.contains("samsung")
                        || name_lower.contains("galaxy")
                        || name_lower.contains("oneplus")
                        || name_lower.contains("xiaomi")
                        || name_lower.contains("huawei")
                        || name_lower.contains("motorola")
                        || name_lower.contains("nokia")
                        || name_lower.contains("lg")
                };

                if is_phone {
                    eprintln!("Found phone: {} ({})", name, address);
                    phones.push(BluetoothDevice {
                        address,
                        name,
                        paired,
                        connected,
                        trusted,
                    });
                }
            }
        }

        Ok(phones)
    }

    /// Get the first paired smartphone (heuristic: has "Phone" or "iPhone" in name or icon)
    /// Prefers connected devices over disconnected ones.
    pub async fn get_first_paired_phone(&self) -> Result<Option<BluetoothDevice>> {
        let phones = self.get_all_paired_phones().await?;

        // Prefer connected phones
        if let Some(connected_phone) = phones.iter().find(|p| p.connected) {
            eprintln!(
                "Found connected phone: {} ({})",
                connected_phone.name, connected_phone.address
            );
            return Ok(Some(connected_phone.clone()));
        }

        // Return first phone if no connected one
        if let Some(phone) = phones.first() {
            eprintln!(
                "Found phone (not connected): {} ({})",
                phone.name, phone.address
            );
            return Ok(Some(phone.clone()));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_device_manager_creation() {
        match DeviceManager::new().await {
            Ok(_manager) => {
                // Successfully created device manager
            }
            Err(_) => {
                // Skip test if D-Bus not available
                eprintln!("D-Bus not available, skipping test");
            }
        }
    }

    #[tokio::test]
    async fn test_get_all_paired_phones() {
        match DeviceManager::new().await {
            Ok(manager) => {
                match manager.get_all_paired_phones().await {
                    Ok(phones) => {
                        // Test passes regardless of phone count - we just verify the function works
                        eprintln!("Found {} paired phones", phones.len());
                        for phone in &phones {
                            assert!(
                                !phone.address.is_empty(),
                                "Phone address should not be empty"
                            );
                            assert!(!phone.name.is_empty(), "Phone name should not be empty");
                            assert!(phone.paired, "Phone should be paired");
                        }
                    }
                    Err(e) => {
                        eprintln!("Error getting paired phones: {} - this may be expected in test environment", e);
                    }
                }
            }
            Err(_) => {
                // Skip test if D-Bus not available
                eprintln!("D-Bus not available, skipping test");
            }
        }
    }

    #[test]
    fn test_bluetooth_device_clone() {
        let device = BluetoothDevice {
            address: "00:11:22:33:44:55".to_string(),
            name: "Test Phone".to_string(),
            paired: true,
            connected: false,
            trusted: true,
        };

        let cloned = device.clone();
        assert_eq!(cloned.address, device.address);
        assert_eq!(cloned.name, device.name);
        assert_eq!(cloned.paired, device.paired);
        assert_eq!(cloned.connected, device.connected);
        assert_eq!(cloned.trusted, device.trusted);
    }

    #[test]
    fn test_bluetooth_device_debug() {
        let device = BluetoothDevice {
            address: "AA:BB:CC:DD:EE:FF".to_string(),
            name: "My iPhone".to_string(),
            paired: true,
            connected: true,
            trusted: false,
        };

        let debug_str = format!("{:?}", device);
        assert!(debug_str.contains("AA:BB:CC:DD:EE:FF"));
        assert!(debug_str.contains("My iPhone"));
    }
}
