use crate::error::Result;
use zbus::{proxy, Connection};

/// BlueZ OBEX Client interface for session management
#[proxy(
    interface = "org.bluez.obex.Client1",
    default_service = "org.bluez.obex",
    default_path = "/org/bluez/obex"
)]
trait ObexClient {
    /// Create a new OBEX session
    ///
    /// # Arguments
    /// * `destination` - Bluetooth device address
    /// * `args` - Session parameters (Target UUID, etc.)
    fn create_session(
        &self,
        destination: &str,
        args: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    /// Remove an OBEX session
    fn remove_session(&self, session: zbus::zvariant::ObjectPath<'_>) -> zbus::Result<()>;
}

/// BlueZ OBEX MessageAccess interface for MAP operations
#[proxy(
    interface = "org.bluez.obex.MessageAccess1",
    default_service = "org.bluez.obex"
)]
trait MessageAccess {
    /// Set current folder (e.g., "telecom/msg/inbox")
    fn set_folder(&self, name: &str) -> zbus::Result<()>;

    /// List messages in current folder
    /// Returns a dict mapping message object paths to their properties
    fn list_messages(
        &self,
        folder: &str,
        filter: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<
        std::collections::HashMap<
            zbus::zvariant::OwnedObjectPath,
            std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        >,
    >;

    /// Get message content by handle
    fn get_message(
        &self,
        handle: &str,
        target_file: &str,
        attachment: bool,
    ) -> zbus::Result<(
        zbus::zvariant::OwnedObjectPath,
        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    )>;

    /// Push message (send SMS)
    fn push_message(
        &self,
        source_file: &str,
        folder: &str,
        args: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<(
        zbus::zvariant::OwnedObjectPath,
        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    )>;

    /// Update message read status
    fn update_inbox(&self, target_file: &str) -> zbus::Result<()>;
}

/// BlueZ OBEX PhonebookAccess interface for PBAP operations
#[proxy(
    interface = "org.bluez.obex.PhonebookAccess1",
    default_service = "org.bluez.obex"
)]
trait PhonebookAccess {
    /// Select phonebook location (e.g., "int" for internal memory, "sim" for SIM card)
    fn select(&self, location: &str, phonebook: &str) -> zbus::Result<()>;

    /// Pull all contacts from current phonebook
    fn pull_all(
        &self,
        target_file: &str,
        filter: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<(
        zbus::zvariant::OwnedObjectPath,
        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    )>;

    /// Pull single vCard by handle
    fn pull(
        &self,
        vcard: &str,
        target_file: &str,
        filter: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<(
        zbus::zvariant::OwnedObjectPath,
        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    )>;

    /// List all vCards in current phonebook
    fn list(
        &self,
        filter: std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<Vec<(String, String)>>;
}

/// BlueZ OBEX Transfer interface for monitoring file transfers
#[proxy(
    interface = "org.bluez.obex.Transfer1",
    default_service = "org.bluez.obex"
)]
trait Transfer {
    /// Cancel ongoing transfer
    fn cancel(&self) -> zbus::Result<()>;

    /// Transfer status property
    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    /// Number of bytes transferred
    #[zbus(property)]
    fn transferred(&self) -> zbus::Result<u64>;

    /// Total transfer size
    #[zbus(property)]
    fn size(&self) -> zbus::Result<u64>;
}

/// BlueZ OBEX Session interface
#[proxy(
    interface = "org.bluez.obex.Session1",
    default_service = "org.bluez.obex"
)]
trait Session {
    /// Session source (adapter address)
    #[zbus(property)]
    fn source(&self) -> zbus::Result<String>;

    /// Session destination (device address)
    #[zbus(property)]
    fn destination(&self) -> zbus::Result<String>;

    /// Session channel
    #[zbus(property)]
    fn channel(&self) -> zbus::Result<u8>;

    /// Session target UUID
    #[zbus(property)]
    fn target(&self) -> zbus::Result<String>;

    /// Session root folder
    #[zbus(property)]
    fn root(&self) -> zbus::Result<String>;
}

/// Helper to create OBEX client connection
pub async fn connect_obex() -> Result<ObexClientProxy<'static>> {
    let connection = Connection::session().await?;
    Ok(ObexClientProxy::new(&connection).await?)
}

/// Helper to create MessageAccess proxy for a session
pub async fn connect_map(session_path: String) -> Result<MessageAccessProxy<'static>> {
    let connection = Connection::session().await?;
    let path: zbus::zvariant::OwnedObjectPath = session_path.try_into()?;
    Ok(MessageAccessProxy::builder(&connection)
        .path(path)?
        .build()
        .await?)
}

/// Helper to create PhonebookAccess proxy for a session
pub async fn connect_pbap(session_path: String) -> Result<PhonebookAccessProxy<'static>> {
    let connection = Connection::session().await?;
    let path: zbus::zvariant::OwnedObjectPath = session_path.try_into()?;
    Ok(PhonebookAccessProxy::builder(&connection)
        .path(path)?
        .build()
        .await?)
}

/// Helper to create Transfer proxy
pub async fn connect_transfer(transfer_path: String) -> Result<TransferProxy<'static>> {
    let connection = Connection::session().await?;
    let path: zbus::zvariant::OwnedObjectPath = transfer_path.try_into()?;
    Ok(TransferProxy::builder(&connection)
        .path(path)?
        .build()
        .await?)
}

/// Helper to create Session proxy
pub async fn connect_session(session_path: String) -> Result<SessionProxy<'static>> {
    let connection = Connection::session().await?;
    let path: zbus::zvariant::OwnedObjectPath = session_path.try_into()?;
    Ok(SessionProxy::builder(&connection)
        .path(path)?
        .build()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_obex_client_connection() {
        // This test requires BlueZ obexd running
        match connect_obex().await {
            Ok(_proxy) => {
                // Successfully connected to OBEX client
            }
            Err(_) => {
                // Skip test if BlueZ not available
                eprintln!("BlueZ obexd not available, skipping test");
            }
        }
    }
}
