//! ANCS over BlueZ GATT (system bus). BlueZ exposes the iPhone's GATT database once the
//! phone is connected and bonded; no scanning or separate BLE stack is needed.

use crate::bluetooth::ancs::{
    AncsNotification, AncsSession, CONTROL_POINT_UUID, DATA_SOURCE_UUID, NOTIFICATION_SOURCE_UUID,
};
use crate::bluetooth::dbus_proxies::{
    BLUEZ_SERVICE, GattCharacteristicProxy, prop_bool, prop_string,
};
use crate::bluetooth::device_manager::device_path_suffix;
use crate::error::{BtsmsError, Result};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use zbus::fdo::ObjectManagerProxy;
use zbus::message::Type as MessageType;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant};

/// Why [`AncsClient::run`] returned.
#[derive(Debug, PartialEq, Eq)]
pub enum AncsStop {
    Cancelled,
    DeviceDisconnected,
}

pub struct AncsClient {
    conn: Connection,
    device_path: OwnedObjectPath,
    notification_source: OwnedObjectPath,
    control_point: OwnedObjectPath,
    data_source: OwnedObjectPath,
}

impl AncsClient {
    /// Locate the ANCS characteristics of the (already connected) iPhone `address`.
    pub async fn connect(address: &str) -> Result<Self> {
        let conn = Connection::system().await?;
        let om = ObjectManagerProxy::builder(&conn)
            .destination(BLUEZ_SERVICE)
            .path("/")
            .build()
            .await?;
        let objects = om.get_managed_objects().await?;
        let suffix = device_path_suffix(address);

        let device_path = objects
            .keys()
            .find(|p| p.as_str().ends_with(&suffix))
            .cloned()
            .ok_or_else(|| BtsmsError::Bluetooth(format!("{address} is not known to BlueZ")))?;

        let mut chars: HashMap<String, OwnedObjectPath> = HashMap::new();
        for (path, ifaces) in &objects {
            if !path.as_str().starts_with(device_path.as_str()) {
                continue;
            }
            if let Some((_, props)) = ifaces
                .iter()
                .find(|(n, _)| n.as_str() == "org.bluez.GattCharacteristic1")
                && let Some(uuid) = prop_string(props, "UUID")
            {
                chars.insert(uuid.to_ascii_lowercase(), path.clone());
            }
        }
        let find = |uuid: &str| {
            chars.get(uuid).cloned().ok_or_else(|| {
                BtsmsError::Bluetooth(
                    "The iPhone does not expose ANCS. Enable 'Show Notifications' for this \
                     computer in Settings > Bluetooth > (i) and make sure the phone is connected."
                        .into(),
                )
            })
        };
        let client = Self {
            notification_source: find(NOTIFICATION_SOURCE_UUID)?,
            control_point: find(CONTROL_POINT_UUID)?,
            data_source: find(DATA_SOURCE_UUID)?,
            device_path,
            conn,
        };
        tracing::info!(
            device = client.device_path.as_str(),
            "ANCS characteristics found"
        );
        Ok(client)
    }

    async fn characteristic(
        &self,
        path: &OwnedObjectPath,
    ) -> Result<GattCharacteristicProxy<'static>> {
        Ok(GattCharacteristicProxy::builder(&self.conn)
            .path(path.clone())
            .build()
            .await?)
    }

    /// Subscribe and forward notifications until cancelled or the phone disconnects.
    pub async fn run(
        self,
        tx: mpsc::Sender<AncsNotification>,
        cancel: CancellationToken,
    ) -> Result<AncsStop> {
        // Subscribe to signals before StartNotify so nothing is missed.
        let rule = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .interface("org.freedesktop.DBus.Properties")
            .member("PropertiesChanged")
            .path_namespace(self.device_path.as_str())
            .build()?;
        let mut stream = MessageStream::for_match_rule(rule, &self.conn, None).await?;

        let ds = self.characteristic(&self.data_source).await?;
        let ns = self.characteristic(&self.notification_source).await?;
        let cp = self.characteristic(&self.control_point).await?;
        ds.start_notify().await?;
        ns.start_notify().await?;
        tracing::info!("ANCS subscribed");

        let mut session = AncsSession::new();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let stop = loop {
            tokio::select! {
                () = cancel.cancelled() => break AncsStop::Cancelled,
                _ = tick.tick() => {
                    if let Some(uid) = session.expire(Instant::now()) {
                        tracing::debug!(uid, "ANCS attribute request timed out");
                    }
                }
                msg = stream.next() => {
                    let Some(Ok(msg)) = msg else {
                        return Err(BtsmsError::Bluetooth("D-Bus stream for ANCS ended".into()));
                    };
                    let header = msg.header();
                    let Some(path) = header.path().map(|p| p.as_str().to_string()) else { continue };
                    let Ok((iface, changed, _)) = msg
                        .body()
                        .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
                    else {
                        continue;
                    };

                    if iface == "org.bluez.Device1" && path == self.device_path.as_str() {
                        if prop_bool(&changed, "Connected") == Some(false) {
                            tracing::info!("phone disconnected, stopping ANCS");
                            break AncsStop::DeviceDisconnected;
                        }
                        continue;
                    }
                    if iface != "org.bluez.GattCharacteristic1" {
                        continue;
                    }
                    let Some(value) = changed.get("Value").and_then(variant_bytes) else { continue };

                    if path == self.notification_source.as_str() {
                        if let Some(event) = session.on_notification_source(&value) {
                            tracing::trace!(?event, "ANCS source event");
                        }
                    } else if path == self.data_source.as_str()
                        && let Some(n) = session.on_data_source(&value)
                    {
                        tracing::debug!(
                            uid = n.uid,
                            app = n.app_identifier.as_deref().unwrap_or("?"),
                            category = ?n.category,
                            "ANCS notification"
                        );
                        if tx.send(n).await.is_err() {
                            break AncsStop::Cancelled;
                        }
                    }
                }
            }
            if let Some(request) = session.next_request(Instant::now())
                && let Err(e) = cp.write_value(&request, HashMap::new()).await
            {
                // 0xA2 "invalid parameter": the notification was removed before we asked.
                if e.to_string().contains("0xa2") {
                    tracing::debug!(error = %e, "ANCS notification gone before attributes were read");
                } else {
                    tracing::warn!(error = %e, "ANCS control point write failed");
                }
                session.request_failed();
            }
        };

        let _ = ns.stop_notify().await;
        let _ = ds.stop_notify().await;
        Ok(stop)
    }
}

fn variant_bytes(v: &OwnedVariant) -> Option<Vec<u8>> {
    v.try_clone().ok().and_then(|v| Vec::<u8>::try_from(v).ok())
}
