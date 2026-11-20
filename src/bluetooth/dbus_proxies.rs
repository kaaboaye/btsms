//! zbus proxies for the parts of BlueZ we use.
//!
//! Signatures follow BlueZ's `doc/org.bluez.obex.*.rst` and `org.bluez.GattCharacteristic.rst`.

use std::collections::HashMap;
use zbus::{OwnedObjectPath, OwnedVariant, Variant, proxy};

/// `a{sv}` argument.
pub type DictArg<'a> = HashMap<&'a str, Variant<'a>>;
/// `a{sv}` result.
pub type PropMap = HashMap<String, OwnedVariant>;

pub const OBEX_SERVICE: &str = "org.bluez.obex";
pub const BLUEZ_SERVICE: &str = "org.bluez";

/// org.bluez.obex.Client1 - creates and removes OBEX sessions.
#[proxy(
    interface = "org.bluez.obex.Client1",
    default_service = "org.bluez.obex",
    default_path = "/org/bluez/obex"
)]
pub trait ObexClient {
    /// `Target` is "map", "pbap", "opp", ...
    fn create_session(&self, destination: &str, args: DictArg<'_>)
    -> zbus::Result<OwnedObjectPath>;

    fn remove_session(&self, session: &zbus::ObjectPath<'_>) -> zbus::Result<()>;
}

/// org.bluez.obex.MessageAccess1 - lives on a MAP session object.
#[proxy(
    interface = "org.bluez.obex.MessageAccess1",
    default_service = "org.bluez.obex"
)]
pub trait MessageAccess {
    /// Absolute ("/" resets to root) or relative folder ("telecom/msg/inbox", "..").
    fn set_folder(&self, name: &str) -> zbus::Result<()>;

    fn list_folders(&self, filter: DictArg<'_>) -> zbus::Result<Vec<PropMap>>;

    fn list_filter_fields(&self) -> zbus::Result<Vec<String>>;

    /// Messages in `folder` (relative to the current folder, "" = current).
    fn list_messages(
        &self,
        folder: &str,
        filter: DictArg<'_>,
    ) -> zbus::Result<HashMap<OwnedObjectPath, PropMap>>;

    /// Ask the phone to check for new messages (no arguments in BlueZ).
    fn update_inbox(&self) -> zbus::Result<()>;

    /// Push a bMessage file to `folder` (relative to the current folder).
    fn push_message(
        &self,
        source_file: &str,
        folder: &str,
        args: DictArg<'_>,
    ) -> zbus::Result<(OwnedObjectPath, PropMap)>;
}

/// org.bluez.obex.Message1 - one object per listed message.
#[proxy(
    interface = "org.bluez.obex.Message1",
    default_service = "org.bluez.obex"
)]
pub trait Message {
    /// Download the message as a bMessage. An empty `target_file` makes obexd create a
    /// temporary file whose path comes back in the `Filename` property.
    fn get(&self, target_file: &str, attachment: bool) -> zbus::Result<(OwnedObjectPath, PropMap)>;

    #[zbus(property)]
    fn read(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_read(&self, value: bool) -> zbus::Result<()>;
}

/// org.bluez.obex.PhonebookAccess1 - lives on a PBAP session object.
#[proxy(
    interface = "org.bluez.obex.PhonebookAccess1",
    default_service = "org.bluez.obex"
)]
pub trait PhonebookAccess {
    /// `location` is "int" or "sim1", `phonebook` is "pb", "ich", "och", ...
    fn select(&self, location: &str, phonebook: &str) -> zbus::Result<()>;

    fn pull_all(
        &self,
        target_file: &str,
        filters: DictArg<'_>,
    ) -> zbus::Result<(OwnedObjectPath, PropMap)>;

    fn get_size(&self) -> zbus::Result<u16>;
}

/// org.bluez.obex.Transfer1 - removed by obexd as soon as the transfer ends.
#[proxy(
    interface = "org.bluez.obex.Transfer1",
    default_service = "org.bluez.obex"
)]
pub trait Transfer {
    fn cancel(&self) -> zbus::Result<()>;

    /// "queued", "active", "suspended", "complete" or "error"
    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;
}

/// org.bluez.Device1
#[proxy(interface = "org.bluez.Device1", default_service = "org.bluez")]
pub trait Device {
    fn connect(&self) -> zbus::Result<()>;

    fn disconnect(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn connected(&self) -> zbus::Result<bool>;
}

/// org.bluez.GattCharacteristic1
#[proxy(
    interface = "org.bluez.GattCharacteristic1",
    default_service = "org.bluez"
)]
pub trait GattCharacteristic {
    fn start_notify(&self) -> zbus::Result<()>;

    fn stop_notify(&self) -> zbus::Result<()>;

    fn write_value(&self, value: &[u8], options: DictArg<'_>) -> zbus::Result<()>;
}

/// Read a string property out of an `a{sv}` map.
pub fn prop_string(props: &PropMap, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| v.downcast_ref::<&str>().ok().map(str::to_string))
}

/// Read a bool property out of an `a{sv}` map.
pub fn prop_bool(props: &PropMap, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| v.downcast_ref::<bool>().ok())
}

/// Read an unsigned integer property (any width) out of an `a{sv}` map.
pub fn prop_u64(props: &PropMap, key: &str) -> Option<u64> {
    let v = props.get(key)?;
    v.downcast_ref::<u64>()
        .ok()
        .or_else(|| v.downcast_ref::<u32>().ok().map(u64::from))
        .or_else(|| v.downcast_ref::<u16>().ok().map(u64::from))
        .or_else(|| v.downcast_ref::<u8>().ok().map(u64::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(v: Variant<'_>) -> OwnedVariant {
        v.try_into_owned().unwrap()
    }

    #[test]
    fn prop_helpers_read_typed_values() {
        let mut props = PropMap::new();
        props.insert("Subject".into(), owned(Variant::from("hi")));
        props.insert("Read".into(), owned(Variant::from(true)));
        props.insert("Size".into(), owned(Variant::from(42u64)));
        props.insert("Small".into(), owned(Variant::from(7u16)));

        assert_eq!(prop_string(&props, "Subject").as_deref(), Some("hi"));
        assert_eq!(prop_bool(&props, "Read"), Some(true));
        assert_eq!(prop_u64(&props, "Size"), Some(42));
        assert_eq!(prop_u64(&props, "Small"), Some(7));
    }

    #[test]
    fn prop_helpers_handle_missing_and_wrong_types() {
        let mut props = PropMap::new();
        props.insert("Read".into(), owned(Variant::from("not a bool")));
        assert_eq!(prop_bool(&props, "Read"), None);
        assert_eq!(prop_string(&props, "Missing"), None);
        assert_eq!(prop_u64(&props, "Read"), None);
    }
}
