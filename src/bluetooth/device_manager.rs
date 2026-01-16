use crate::error::{BtsmsError, Result};
use zbus::Connection;
use std::collections::HashMap;

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
        let objects: HashMap<zbus::zvariant::OwnedObjectPath, HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>> =
            proxy.call("GetManagedObjects", &()).await?;

        let mut devices = Vec::new();

        for (_path, interfaces) in objects {
            // Check if this object is a Bluetooth device
            if let Some(device_props) = interfaces.get("org.bluez.Device1") {
                // Get device properties
                let address = device_props
                    .get("Address")
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .map(|s| s.clone())
                    .unwrap_or_default();

                let name = device_props
                    .get("Name")
                    .or_else(|| device_props.get("Alias"))
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .map(|s| s.clone())
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
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .map(|s| s.clone());

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

        let objects: HashMap<zbus::zvariant::OwnedObjectPath, HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>> =
            proxy.call("GetManagedObjects", &()).await?;

        for (path, interfaces) in objects {
            if let Some(device_props) = interfaces.get("org.bluez.Device1") {
                let address = device_props
                    .get("Address")
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .map(|s| s.clone())
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

        let objects: HashMap<zbus::zvariant::OwnedObjectPath, HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>> =
            proxy.call("GetManagedObjects", &()).await?;

        for (path, interfaces) in objects {
            if let Some(device_props) = interfaces.get("org.bluez.Device1") {
                let address = device_props
                    .get("Address")
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .map(|s| s.clone())
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

    /// Get the first paired smartphone (heuristic: has "Phone" or "iPhone" in name or icon)
    pub async fn get_first_paired_phone(&self) -> Result<Option<BluetoothDevice>> {
        // Get all paired devices with their icons
        let proxy = zbus::Proxy::new(
            &self.connection,
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
        )
        .await?;

        let objects: HashMap<zbus::zvariant::OwnedObjectPath, HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>> =
            proxy.call("GetManagedObjects", &()).await?;

        // Try to find a phone by icon or name
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
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .map(|s| s.clone());

                let name = device_props
                    .get("Name")
                    .or_else(|| device_props.get("Alias"))
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .map(|s| s.clone())
                    .unwrap_or_else(|| "Unknown Device".to_string());

                let address = device_props
                    .get("Address")
                    .and_then(|v| v.downcast_ref::<String>().ok())
                    .map(|s| s.clone())
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
                if let Some(icon_str) = &icon {
                    if icon_str == "phone" {
                        eprintln!("Found phone by icon: {} ({})", name, address);
                        return Ok(Some(BluetoothDevice {
                            address,
                            name,
                            paired,
                            connected,
                            trusted,
                        }));
                    }
                }

                // Check by name
                let name_lower = name.to_lowercase();
                if name_lower.contains("phone")
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
                    || name_lower.contains("lg") {
                    eprintln!("Found phone by name: {} ({})", name, address);
                    return Ok(Some(BluetoothDevice {
                        address,
                        name,
                        paired,
                        connected,
                        trusted,
                    }));
                }
            }
        }

        // If no phone found, return None
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
}
