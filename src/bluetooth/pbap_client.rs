//! PBAP (Phonebook Access Profile) client on top of BlueZ obexd.

use crate::bluetooth::dbus_proxies::{DictArg, PhonebookAccessProxy, prop_string};
use crate::bluetooth::obex::ObexSession;
use crate::error::{BtsmsError, Result, dbus_error_name, is_unknown_object};
use std::collections::HashMap;
use std::time::Duration;
use zbus::Variant;

const SESSION_READY_TIMEOUT: Duration = Duration::from_secs(20);
/// Large phonebooks over Bluetooth take a while.
const PULL_TIMEOUT: Duration = Duration::from_secs(180);
/// vCard properties requested from the phone.
const PHONEBOOK_FIELDS: [&str; 7] = ["VERSION", "FN", "N", "TEL", "EMAIL", "ORG", "UID"];

pub struct PbapClient {
    device_address: String,
    session: Option<ObexSession>,
}

impl PbapClient {
    pub fn new(device_address: String) -> Self {
        Self {
            device_address,
            session: None,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.session.is_some()
    }

    /// Open the PBAP session. iPhone needs "Sync Contacts" enabled for this computer in
    /// Settings > Bluetooth > (i).
    pub async fn connect(&mut self) -> Result<()> {
        if self.session.is_some() {
            return Ok(());
        }
        let session = ObexSession::open(&self.device_address, "pbap").await?;
        wait_for_session_ready(&session).await?;
        self.session = Some(session);
        Ok(())
    }

    pub async fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            session.close().await;
        }
    }

    async fn phonebook(&self) -> Result<(&ObexSession, PhonebookAccessProxy<'static>)> {
        let session = self.session.as_ref().ok_or(BtsmsError::NotConnected)?;
        let proxy = PhonebookAccessProxy::builder(session.connection())
            .path(session.path().clone())
            .build()
            .await?;
        Ok((session, proxy))
    }

    /// Download the main phonebook ("int"/"pb") as vCard 3.0 text.
    pub async fn pull_all_contacts(&self) -> Result<String> {
        let (session, pb) = self.phonebook().await?;
        pb.select("int", "pb").await.map_err(pbap_error)?;
        if let Ok(size) = pb.get_size().await {
            tracing::info!(size, "phonebook size reported by phone");
        }

        let mut filters: DictArg<'_> = HashMap::new();
        filters.insert("Format", Variant::from("vcard30"));
        // Only what we store; without this phones send contact photos (MBs, many seconds).
        filters.insert("Fields", Variant::from(PHONEBOOK_FIELDS.to_vec()));

        let watcher = session.watch_transfers().await?;
        let (transfer, props) = pb.pull_all("", filters).await.map_err(pbap_error)?;
        let file = prop_string(&props, "Filename").map(std::path::PathBuf::from);
        let started = std::time::Instant::now();
        let content = async {
            watcher
                .wait(&transfer, file.as_deref(), PULL_TIMEOUT)
                .await?;
            let path = file.as_ref().ok_or_else(|| {
                BtsmsError::Bluetooth("obexd did not report where it stored the phonebook".into())
            })?;
            let raw = tokio::fs::read(path).await?;
            Ok::<_, BtsmsError>(String::from_utf8_lossy(&raw).into_owned())
        }
        .await;
        if let Some(path) = &file {
            let _ = std::fs::remove_file(path);
        }
        if let Ok(text) = &content {
            tracing::info!(
                bytes = text.len(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "phonebook downloaded"
            );
        }
        content
    }
}

async fn wait_for_session_ready(session: &ObexSession) -> Result<()> {
    let deadline = tokio::time::Instant::now() + SESSION_READY_TIMEOUT;
    let proxy = PhonebookAccessProxy::builder(session.connection())
        .path(session.path().clone())
        .build()
        .await?;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match proxy.select("int", "pb").await {
            Ok(()) => {
                tracing::debug!(attempts, "PBAP session ready");
                return Ok(());
            }
            Err(e) if is_unknown_object(&e) && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Err(e) => {
                tracing::warn!(attempts, error = %e, "PBAP session did not become ready");
                return Err(pbap_error(e));
            }
        }
    }
}

fn pbap_error(err: zbus::Error) -> BtsmsError {
    let name = dbus_error_name(&err).unwrap_or_default();
    if name.ends_with("Forbidden")
        || err.to_string().contains("Permission denied")
        || is_unknown_object(&err)
    {
        return BtsmsError::Bluetooth(
            "The phone did not allow contact access. iPhone: Settings > Bluetooth > (i) next \
             to this computer > enable 'Sync Contacts'. Android: allow contact sharing for \
             this computer in the phone's Bluetooth settings."
                .into(),
        );
    }
    BtsmsError::DBus(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_client_is_disconnected() {
        let client = PbapClient::new("AA:BB:CC:DD:EE:FF".into());
        assert!(!client.is_connected());
    }

    #[test]
    fn permission_errors_explain_what_to_do() {
        let err: zbus::Error = zbus::fdo::Error::UnknownObject("gone".into()).into();
        assert!(pbap_error(err).to_string().contains("Sync Contacts"));
        let other = pbap_error(zbus::Error::Failure("x".into()));
        assert!(matches!(other, BtsmsError::DBus(_)));
    }
}
