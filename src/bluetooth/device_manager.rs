//! Paired device discovery and connection through BlueZ (`org.bluez`, system bus).

use crate::bluetooth::dbus_proxies::{
    BLUEZ_SERVICE, DeviceProxy, PropMap, prop_bool, prop_string, prop_u64,
};
use crate::error::{BtsmsError, Result};
use zbus::Connection;
use zbus::fdo::ObjectManagerProxy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
    /// D-Bus object path, e.g. `/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`.
    pub path: String,
    pub icon: Option<String>,
    pub class: Option<u32>,
    pub paired: bool,
    pub connected: bool,
    pub trusted: bool,
}

impl BluetoothDevice {
    /// Build from the `org.bluez.Device1` properties of a managed object.
    pub fn from_props(path: &str, props: &PropMap) -> Option<Self> {
        let address = prop_string(props, "Address")?;
        let name = prop_string(props, "Alias")
            .or_else(|| prop_string(props, "Name"))
            .unwrap_or_else(|| address.clone());
        Some(Self {
            address,
            name,
            path: path.to_string(),
            icon: prop_string(props, "Icon"),
            class: prop_u64(props, "Class").map(|c| c as u32),
            paired: prop_bool(props, "Paired").unwrap_or(false),
            connected: prop_bool(props, "Connected").unwrap_or(false),
            trusted: prop_bool(props, "Trusted").unwrap_or(false),
        })
    }

    /// Phones report icon "phone" and Class-of-Device major class 2.
    pub fn is_phone(&self) -> bool {
        if self.icon.as_deref() == Some("phone") {
            return true;
        }
        if let Some(class) = self.class {
            return major_device_class(class) == MAJOR_CLASS_PHONE;
        }
        // No icon/class (LE-only or not fully discovered): fall back to the name.
        let name = self.name.to_lowercase();
        ["iphone", "android", "pixel", "galaxy", "phone"]
            .iter()
            .any(|n| name.contains(n))
    }
}

const MAJOR_CLASS_PHONE: u32 = 0x02;

/// Major device class bits of a Bluetooth Class of Device.
pub fn major_device_class(class: u32) -> u32 {
    (class >> 8) & 0x1f
}

/// BlueZ object path for a device address on adapter `hci0`-style paths.
pub fn device_path_suffix(address: &str) -> String {
    format!("dev_{}", address.to_ascii_uppercase().replace(':', "_"))
}

pub struct DeviceManager {
    connection: Connection,
}

impl DeviceManager {
    pub async fn new() -> Result<Self> {
        let connection = Connection::system().await?;
        Ok(Self { connection })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Every device BlueZ knows about.
    pub async fn all_devices(&self) -> Result<Vec<BluetoothDevice>> {
        let om = ObjectManagerProxy::builder(&self.connection)
            .destination(BLUEZ_SERVICE)
            .path("/")
            .build()
            .await?;
        let objects = om.get_managed_objects().await.map_err(|e| {
            BtsmsError::Bluetooth(format!(
                "Cannot talk to BlueZ ({e}). Is the bluetooth service running?"
            ))
        })?;
        let mut devices: Vec<BluetoothDevice> = objects
            .iter()
            .filter_map(|(path, ifaces)| {
                ifaces
                    .iter()
                    .find(|(name, _)| name.as_str() == "org.bluez.Device1")
                    .and_then(|(_, props)| BluetoothDevice::from_props(path.as_str(), props))
            })
            .collect();
        devices.sort_by(|a, b| a.name.cmp(&b.name).then(a.address.cmp(&b.address)));
        Ok(devices)
    }

    pub async fn get_paired_devices(&self) -> Result<Vec<BluetoothDevice>> {
        Ok(self
            .all_devices()
            .await?
            .into_iter()
            .filter(|d| d.paired)
            .collect())
    }

    pub async fn get_all_paired_phones(&self) -> Result<Vec<BluetoothDevice>> {
        let phones: Vec<_> = self
            .get_paired_devices()
            .await?
            .into_iter()
            .filter(BluetoothDevice::is_phone)
            .collect();
        tracing::debug!(count = phones.len(), "paired phones");
        Ok(phones)
    }

    /// First paired phone, preferring a connected one.
    pub async fn get_first_paired_phone(&self) -> Result<Option<BluetoothDevice>> {
        let phones = self.get_all_paired_phones().await?;
        Ok(phones
            .iter()
            .find(|p| p.connected)
            .or_else(|| phones.first())
            .cloned())
    }

    pub async fn find_device(&self, address: &str) -> Result<BluetoothDevice> {
        self.all_devices()
            .await?
            .into_iter()
            .find(|d| d.address.eq_ignore_ascii_case(address))
            .ok_or_else(|| BtsmsError::Bluetooth(format!("Device {address} is not known to BlueZ")))
    }

    async fn device_proxy(&self, address: &str) -> Result<DeviceProxy<'static>> {
        let device = self.find_device(address).await?;
        Ok(DeviceProxy::builder(&self.connection)
            .path(zbus::OwnedObjectPath::try_from(device.path)?)
            .build()
            .await?)
    }

    pub async fn is_connected(&self, address: &str) -> Result<bool> {
        Ok(self.device_proxy(address).await?.connected().await?)
    }

    pub async fn connect_device(&self, address: &str) -> Result<()> {
        tracing::info!(address, "connecting Bluetooth device");
        self.device_proxy(address)
            .await?
            .connect()
            .await
            .map_err(|e| BtsmsError::Bluetooth(format!("Could not connect to {address}: {e}")))
    }

    pub async fn disconnect_device(&self, address: &str) -> Result<()> {
        tracing::info!(address, "disconnecting Bluetooth device");
        self.device_proxy(address).await?.disconnect().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::Variant;

    fn device(name: &str, icon: Option<&str>, class: Option<u32>) -> BluetoothDevice {
        BluetoothDevice {
            address: "AA:BB:CC:DD:EE:FF".into(),
            name: name.into(),
            path: String::new(),
            icon: icon.map(str::to_string),
            class,
            paired: true,
            connected: false,
            trusted: false,
        }
    }

    #[test]
    fn phones_are_recognised_by_icon_and_class() {
        assert!(device("iThe", Some("phone"), None).is_phone());
        // iPhone class 0x7a020c: major class 2 (phone), minor smartphone
        assert!(device("whatever", None, Some(0x007a_020c)).is_phone());
    }

    #[test]
    fn headphones_are_not_phones() {
        // Old code matched "lg" in any name, so e.g. "theGlos" counted as an LG phone.
        assert!(!device("theGlos", Some("audio-card"), Some(0x240404)).is_phone());
        assert!(!device("LG Speaker", Some("audio-card"), None).is_phone());
    }

    #[test]
    fn name_fallback_without_icon_or_class() {
        assert!(device("Jan's iPhone", None, None).is_phone());
        assert!(!device("Keyboard", None, None).is_phone());
    }

    #[test]
    fn major_class_bits() {
        assert_eq!(major_device_class(0x007a_020c), 2);
        assert_eq!(major_device_class(0x0024_0404), 4);
    }

    #[test]
    fn device_path_suffix_format() {
        assert_eq!(
            device_path_suffix("aa:bb:cc:dd:ee:01"),
            "dev_AA_BB_CC_DD_EE_01"
        );
    }

    #[test]
    fn device_from_props() {
        let mut props = PropMap::new();
        let ov = |v: Variant<'_>| v.try_into_owned().unwrap();
        props.insert("Address".into(), ov(Variant::from("AA:BB:CC:DD:EE:FF")));
        props.insert("Alias".into(), ov(Variant::from("My Phone")));
        props.insert("Icon".into(), ov(Variant::from("phone")));
        props.insert("Class".into(), ov(Variant::from(0x5a020cu32)));
        props.insert("Paired".into(), ov(Variant::from(true)));
        props.insert("Connected".into(), ov(Variant::from(true)));
        let d = BluetoothDevice::from_props("/org/bluez/hci0/dev_AA", &props).unwrap();
        assert_eq!(d.name, "My Phone");
        assert_eq!(d.class, Some(0x5a020c));
        assert!(d.paired && d.connected && !d.trusted);
        assert!(d.is_phone());

        props.remove("Address");
        assert!(BluetoothDevice::from_props("/x", &props).is_none());
    }
}
