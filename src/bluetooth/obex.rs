//! OBEX session handling shared by the MAP and PBAP clients.
//!
//! obexd ties every session to the D-Bus connection that created it and removes it when that
//! connection goes away, so each session owns a private session-bus connection: dropping the
//! session (even without calling [`ObexSession::close`]) cleans up on the obexd side.

use crate::bluetooth::dbus_proxies::{
    DictArg, OBEX_SERVICE, ObexClientProxy, PropMap, TransferProxy,
};
use crate::error::{BtsmsError, Result, dbus_error_name, is_unknown_object};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use zbus::message::Type as MessageType;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant};

/// An open OBEX session (MAP or PBAP) with a phone.
pub struct ObexSession {
    conn: Connection,
    path: OwnedObjectPath,
    target: &'static str,
    device: String,
}

impl std::fmt::Debug for ObexSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObexSession")
            .field("path", &self.path.as_str())
            .field("target", &self.target)
            .field("device", &self.device)
            .finish()
    }
}

impl ObexSession {
    /// Create a session for `target` ("map" or "pbap") with `device`.
    pub async fn open(device: &str, target: &'static str) -> Result<Self> {
        let conn = Connection::session().await?;
        ensure_obexd_running(&conn).await;

        let client = ObexClientProxy::new(&conn).await?;
        let mut args: DictArg<'_> = HashMap::new();
        args.insert("Target", Variant::from(target));

        tracing::debug!(device, target, "creating OBEX session");
        let started = std::time::Instant::now();
        let path = client
            .create_session(device, args)
            .await
            .map_err(|e| session_error(target, e))?;
        tracing::info!(
            device,
            target,
            session = path.as_str(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "OBEX session created"
        );

        Ok(Self {
            conn,
            path,
            target,
            device: device.to_string(),
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn path(&self) -> &OwnedObjectPath {
        &self.path
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// Remove the session on the obexd side.
    pub async fn close(self) {
        match ObexClientProxy::new(&self.conn).await {
            Ok(client) => match client.remove_session(&self.path).await {
                Ok(()) => tracing::debug!(session = self.path.as_str(), "OBEX session removed"),
                Err(e) => tracing::debug!(
                    session = self.path.as_str(),
                    error = %e,
                    "removing OBEX session failed (probably already gone)"
                ),
            },
            Err(e) => tracing::debug!(error = %e, "no OBEX client proxy while closing"),
        }
    }

    /// Subscribe to transfer status changes. Call this *before* starting the transfer so a
    /// transfer that finishes instantly is not missed.
    pub async fn watch_transfers(&self) -> Result<TransferWatcher> {
        let rule = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .interface("org.freedesktop.DBus.Properties")
            .member("PropertiesChanged")
            .path_namespace(self.path.as_str())
            .build()?;
        let stream = MessageStream::for_match_rule(rule, &self.conn, None).await?;
        Ok(TransferWatcher {
            conn: self.conn.clone(),
            stream,
        })
    }
}

/// Waits for an obexd transfer to finish.
pub struct TransferWatcher {
    conn: Connection,
    stream: MessageStream,
}

/// Outcome of a transfer status signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferState {
    Pending,
    Complete,
    Error,
}

/// Map an obexd `Status` string to a [`TransferState`].
pub fn transfer_state(status: &str) -> TransferState {
    match status {
        "complete" => TransferState::Complete,
        "error" => TransferState::Error,
        _ => TransferState::Pending,
    }
}

impl TransferWatcher {
    /// Wait until `transfer` completes. `file` is the file obexd reads or writes; it is used
    /// as a fallback check when the transfer object disappears without a final signal.
    pub async fn wait(
        mut self,
        transfer: &OwnedObjectPath,
        file: Option<&Path>,
        timeout: Duration,
    ) -> Result<()> {
        let started = std::time::Instant::now();
        let wait_signals = async {
            while let Some(msg) = self.stream.next().await {
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "bad message on transfer stream");
                        continue;
                    }
                };
                let header = msg.header();
                if header.path().map(|p| p.as_str()) != Some(transfer.as_str()) {
                    continue;
                }
                let body = msg.body();
                let Ok((iface, changed, _invalidated)) =
                    body.deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
                else {
                    continue;
                };
                if iface != "org.bluez.obex.Transfer1" {
                    continue;
                }
                let changed: PropMap = changed;
                if let Some(status) =
                    crate::bluetooth::dbus_proxies::prop_string(&changed, "Status")
                {
                    tracing::trace!(transfer = transfer.as_str(), status, "transfer status");
                    match transfer_state(&status) {
                        TransferState::Complete => return Ok(()),
                        TransferState::Error => {
                            return Err(BtsmsError::Bluetooth(
                                "OBEX transfer failed on the phone side".into(),
                            ));
                        }
                        TransferState::Pending => {}
                    }
                }
            }
            Err(BtsmsError::Bluetooth(
                "D-Bus connection closed during transfer".into(),
            ))
        };

        let result = match tokio::time::timeout(timeout, wait_signals).await {
            Ok(r) => r,
            Err(_) => self.fallback_check(transfer, file).await,
        };
        tracing::debug!(
            transfer = transfer.as_str(),
            ok = result.is_ok(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "transfer finished"
        );
        result
    }

    /// No final signal arrived in time: ask obexd directly.
    async fn fallback_check(&self, transfer: &OwnedObjectPath, file: Option<&Path>) -> Result<()> {
        let proxy = TransferProxy::builder(&self.conn)
            .path(transfer.clone())
            .build()
            .await?;
        match proxy.status().await {
            Ok(status) if transfer_state(&status) == TransferState::Complete => Ok(()),
            Ok(status) => {
                let _ = proxy.cancel().await;
                Err(BtsmsError::Timeout(format!(
                    "OBEX transfer still '{status}', cancelled"
                )))
            }
            Err(e) if is_unknown_object(&e) => {
                // Transfer object is gone: obexd finished it. Trust the file if we have one.
                match file.map(std::fs::metadata) {
                    Some(Ok(meta)) if meta.len() > 0 => Ok(()),
                    Some(_) => Err(BtsmsError::Bluetooth(
                        "OBEX transfer ended without data".into(),
                    )),
                    None => Ok(()),
                }
            }
            Err(e) => Err(e.into()),
        }
    }
}

/// obexd is D-Bus activated; make sure it is up so the first call does not fail.
async fn ensure_obexd_running(conn: &Connection) {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return;
    };
    let Ok(name) = zbus::names::WellKnownName::try_from(OBEX_SERVICE) else {
        return;
    };
    match dbus.start_service_by_name(name, 0).await {
        Ok(reply) => tracing::trace!(?reply, "obexd activation"),
        Err(e) => tracing::warn!(
            error = %e,
            "could not activate obexd (org.bluez.obex); is BlueZ's obex service installed?"
        ),
    }
}

/// Turn CreateSession failures into actionable messages.
fn session_error(target: &str, err: zbus::Error) -> BtsmsError {
    let name = dbus_error_name(&err).unwrap_or_default();
    let text = err.to_string();
    tracing::warn!(target, error_name = %name, error = %text, "CreateSession failed");
    let profile = target.to_ascii_uppercase();
    if name.ends_with("ServiceUnknown") {
        return BtsmsError::Bluetooth(
            "obexd is not running and could not be started. Install BlueZ's obex support \
             (e.g. the bluez-obex package) and run: systemctl --user start obex"
                .into(),
        );
    }
    if text.contains("Unable to find service record") {
        return BtsmsError::Bluetooth(format!(
            "The phone does not offer {profile}. iPhone: Settings > Bluetooth > (i) next to this \
             computer > enable 'Show Notifications' (and 'Sync Contacts' for contacts). \
             Android: allow message/contact access when asked."
        ));
    }
    if text.contains("Host is down") || text.contains("No route") || text.contains("not connected")
    {
        return BtsmsError::Bluetooth(format!(
            "The phone is out of range or Bluetooth is off ({text})"
        ));
    }
    if text.contains("Connection refused") {
        return BtsmsError::Bluetooth(format!(
            "The phone refused the {profile} connection. iPhones allow only one {profile} \
             session at a time: close other BT SMS windows or 'btsms-cli listen' and try \
             again. Otherwise unlock the phone and check its Bluetooth settings for this computer."
        ));
    }
    if name.ends_with("Forbidden") || text.contains("Permission denied") {
        return BtsmsError::Bluetooth(format!(
            "The phone refused {profile} access. Unlock the phone and accept the request, or \
             enable access in the phone's Bluetooth settings for this computer."
        ));
    }
    BtsmsError::DBus(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_states() {
        assert_eq!(transfer_state("complete"), TransferState::Complete);
        assert_eq!(transfer_state("error"), TransferState::Error);
        assert_eq!(transfer_state("queued"), TransferState::Pending);
        assert_eq!(transfer_state("active"), TransferState::Pending);
        assert_eq!(transfer_state("suspended"), TransferState::Pending);
    }

    #[test]
    fn session_errors_are_actionable() {
        let err = session_error(
            "map",
            zbus::Error::Failure("Unable to find service record".into()),
        );
        assert!(err.to_string().contains("Show Notifications"));

        let err = session_error(
            "map",
            zbus::Error::Failure("connect to AA: Connection refused (111)".into()),
        );
        assert!(err.to_string().contains("only one MAP session"));

        let err = session_error("pbap", zbus::Error::Failure("Host is down".into()));
        assert!(err.to_string().contains("out of range"));

        let err = session_error("map", zbus::Error::Failure("something odd".into()));
        assert!(matches!(err, BtsmsError::DBus(_)));
    }
}
