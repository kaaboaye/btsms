//! Long-running connection to one phone: keeps the MAP session alive, reconnects with
//! backoff, listens to ANCS (iPhone) to sync the moment a message arrives, polls otherwise,
//! and sends messages. The GUI and `btsms-cli listen` both drive this.

use crate::bluetooth::{
    AncsClient, AncsNotification, AncsStop, BluetoothDevice, DeviceManager, MapClient, PbapClient,
};
use crate::contacts::{ContactManager, normalize_address, normalize_for_sending};
use crate::db::{self, Message, MessageDirection, NewMessage};
use crate::error::{BtsmsError, Result};
use crate::sync::MessageSyncService;
use chrono::Utc;
use sqlx::SqlitePool;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// After an ANCS "new message", wait this long before listing the inbox: the phone needs a
/// moment before MAP shows the message.
const ANCS_SYNC_DELAY: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkSettings {
    pub polling_enabled: bool,
    pub poll_interval: Duration,
}

impl LinkSettings {
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            polling_enabled: config.message_polling_enabled,
            poll_interval: Duration::from_secs(u64::from(config.message_polling_interval)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    Connected {
        /// Real-time notifications (iPhone ANCS) are active.
        live_notifications: bool,
    },
    Reconnecting {
        attempt: u32,
        retry_in: Duration,
        reason: String,
    },
    Stopped,
}

#[derive(Debug, Clone)]
pub enum LinkEvent {
    State(LinkState),
    /// New messages were stored. `initial` is set for the first sync after starting, which
    /// may import a backlog that should not trigger desktop notifications.
    NewMessages {
        messages: Vec<Message>,
        initial: bool,
    },
    SyncFailed(String),
}

enum Command {
    SyncNow,
    Send {
        recipient: String,
        text: String,
        reply: oneshot::Sender<Result<Message>>,
    },
}

/// Handle to a running link. Dropping it stops the link.
pub struct PhoneLink {
    device: BluetoothDevice,
    handle: LinkHandle,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

/// Cloneable sender for commands to a running link.
#[derive(Clone)]
pub struct LinkHandle {
    cmd_tx: mpsc::Sender<Command>,
    /// Latest settings win; a watch channel never drops the final value.
    settings_tx: Arc<watch::Sender<LinkSettings>>,
}

impl LinkHandle {
    /// Send an SMS and store it. Fails when the phone is not connected right now.
    pub async fn send_sms(&self, recipient: &str, text: &str) -> Result<Message> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Send {
                recipient: recipient.to_string(),
                text: text.to_string(),
                reply,
            })
            .await
            .map_err(|_| BtsmsError::NotConnected)?;
        rx.await.map_err(|_| BtsmsError::NotConnected)?
    }

    pub fn sync_now(&self) {
        let _ = self.cmd_tx.try_send(Command::SyncNow);
    }

    pub fn update_settings(&self, settings: LinkSettings) {
        self.settings_tx.send_replace(settings);
    }
}

impl PhoneLink {
    /// Start the link task on the current Tokio runtime.
    pub fn start(
        device: BluetoothDevice,
        pool: SqlitePool,
        settings: LinkSettings,
    ) -> (Self, mpsc::UnboundedReceiver<LinkEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (settings_tx, settings_rx) = watch::channel(settings);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let worker = Worker {
            device: device.clone(),
            pool,
            settings,
            settings_rx,
            settings_open: true,
            cmd_rx,
            events: event_tx,
            cancel: cancel.clone(),
            initial_sync_done: false,
        };
        let span = tracing::info_span!("link", device = %device.address);
        let task = tokio::spawn(tracing::Instrument::instrument(worker.run(), span));
        (
            Self {
                device,
                handle: LinkHandle {
                    cmd_tx,
                    settings_tx: Arc::new(settings_tx),
                },
                cancel,
                task: Some(task),
            },
            event_rx,
        )
    }

    pub fn device(&self) -> &BluetoothDevice {
        &self.device
    }

    pub fn handle(&self) -> LinkHandle {
        self.handle.clone()
    }

    pub async fn send_sms(&self, recipient: &str, text: &str) -> Result<Message> {
        self.handle.send_sms(recipient, text).await
    }

    pub fn sync_now(&self) {
        self.handle.sync_now();
    }

    pub fn update_settings(&self, settings: LinkSettings) {
        self.handle.update_settings(settings);
    }

    /// Stop the link and wait until the MAP session is closed.
    pub async fn stop(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for PhoneLink {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Backoff between reconnect attempts.
pub fn reconnect_delay(attempt: u32) -> Duration {
    const STEPS: [u64; 6] = [2, 5, 10, 20, 40, 60];
    Duration::from_secs(STEPS[(attempt.saturating_sub(1) as usize).min(STEPS.len() - 1)])
}

enum Outcome {
    Stop,
    Lost(String),
}

/// Run `fut` unless `cancel` fires first.
async fn or_cancel<T>(cancel: &CancellationToken, fut: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        () = cancel.cancelled() => None,
        v = fut => Some(v),
    }
}

struct Worker {
    device: BluetoothDevice,
    pool: SqlitePool,
    settings: LinkSettings,
    settings_rx: watch::Receiver<LinkSettings>,
    /// False once every handle is gone (then `changed()` would return immediately forever).
    settings_open: bool,
    cmd_rx: mpsc::Receiver<Command>,
    events: mpsc::UnboundedSender<LinkEvent>,
    cancel: CancellationToken,
    initial_sync_done: bool,
}

impl Worker {
    fn emit(&self, event: LinkEvent) {
        let _ = self.events.send(event);
    }

    /// Apply a settings change, or note that no more changes can come.
    fn take_settings(&mut self, changed: std::result::Result<(), watch::error::RecvError>) {
        match changed {
            Ok(()) => {
                self.settings = *self.settings_rx.borrow_and_update();
                tracing::debug!(settings = ?self.settings, "link settings updated");
            }
            Err(_) => self.settings_open = false,
        }
    }

    async fn run(mut self) {
        let address = self.device.address.clone();
        let mut attempt = 0u32;
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            self.emit(LinkEvent::State(LinkState::Connecting));
            let cancel = self.cancel.clone();
            let reason = match or_cancel(&cancel, connect_map(&self.device)).await {
                None => break,
                Some(Ok(map)) => {
                    attempt = 0;
                    match self.connected(map).await {
                        Outcome::Stop => break,
                        Outcome::Lost(reason) => reason,
                    }
                }
                Some(Err(e)) => e.to_string(),
            };
            attempt += 1;
            let retry_in = reconnect_delay(attempt);
            tracing::warn!(device = %address, attempt, ?retry_in, reason, "phone link lost");
            self.emit(LinkEvent::State(LinkState::Reconnecting {
                attempt,
                retry_in,
                reason,
            }));
            if !self.idle(retry_in).await {
                break;
            }
        }
        tracing::info!(device = %address, "phone link stopped");
        self.emit(LinkEvent::State(LinkState::Stopped));
    }

    /// Wait while disconnected, answering commands. Returns false when cancelled.
    async fn idle(&mut self, duration: Duration) -> bool {
        let deadline = Instant::now() + duration;
        loop {
            tokio::select! {
                () = self.cancel.cancelled() => return false,
                () = tokio::time::sleep_until(deadline) => return true,
                changed = self.settings_rx.changed(), if self.settings_open => {
                    self.take_settings(changed);
                }
                cmd = self.cmd_rx.recv() => match cmd {
                    None => return false,
                    Some(Command::Send { reply, .. }) => {
                        let _ = reply.send(Err(BtsmsError::NotConnected));
                    }
                    Some(Command::SyncNow) => return true,
                },
            }
        }
    }

    async fn connected(&mut self, mut map: MapClient) -> Outcome {
        let address = self.device.address.clone();
        let ancs_cancel = self.cancel.child_token();
        let (ancs_tx, mut ancs_rx) = mpsc::channel::<AncsNotification>(32);
        let cancel = self.cancel.clone();
        let mut ancs_task = match or_cancel(&cancel, AncsClient::connect(&address)).await {
            None => {
                map.disconnect().await;
                return Outcome::Stop;
            }
            Some(Ok(client)) => Some(tokio::spawn(client.run(ancs_tx, ancs_cancel.clone()))),
            Some(Err(e)) => {
                tracing::info!(error = %e, "no ANCS on this phone, relying on polling");
                None
            }
        };
        let live = ancs_task.is_some();
        self.emit(LinkEvent::State(LinkState::Connected {
            live_notifications: live,
        }));
        tracing::info!(device = %address, live_notifications = live, "phone link up");

        let outcome = async {
            match or_cancel(&cancel, self.sync(&map)).await {
                None => return Outcome::Stop,
                Some(Err(reason)) => return Outcome::Lost(reason),
                Some(Ok(())) => {}
            }
            let mut next_poll = Instant::now() + self.settings.poll_interval;
            let mut ancs_sync_at: Option<Instant> = None;
            loop {
                let poll_at = if self.settings.polling_enabled {
                    next_poll
                } else {
                    Instant::now() + Duration::from_secs(86_400)
                };
                let wake = ancs_sync_at.map_or(poll_at, |t| t.min(poll_at));
                tokio::select! {
                    () = self.cancel.cancelled() => return Outcome::Stop,
                    () = tokio::time::sleep_until(wake) => {
                        let now = Instant::now();
                        // A poll that happens to run just before a pending ANCS sync must
                        // not cancel it: MAP may not list the new message yet.
                        if ancs_sync_at.is_some_and(|t| t <= now) {
                            ancs_sync_at = None;
                        }
                        next_poll = now + self.settings.poll_interval;
                        match or_cancel(&cancel, self.sync(&map)).await {
                            None => return Outcome::Stop,
                            Some(Err(reason)) => return Outcome::Lost(reason),
                            Some(Ok(())) => {}
                        }
                    }
                    changed = self.settings_rx.changed(), if self.settings_open => {
                        self.take_settings(changed);
                        next_poll = Instant::now() + self.settings.poll_interval;
                    }
                    cmd = self.cmd_rx.recv() => match cmd {
                        None => return Outcome::Stop,
                        Some(Command::SyncNow) => {
                            next_poll = Instant::now() + self.settings.poll_interval;
                            match or_cancel(&cancel, self.sync(&map)).await {
                                None => return Outcome::Stop,
                                Some(Err(reason)) => return Outcome::Lost(reason),
                                Some(Ok(())) => {}
                            }
                        }
                        Some(Command::Send { recipient, text, reply }) => {
                            let result = send_and_store(&map, &self.pool, &recipient, &text).await;
                            let failed = result.is_err();
                            let _ = reply.send(result);
                            if failed && map.ping().await.is_err() {
                                return Outcome::Lost("MAP session lost while sending".into());
                            }
                        }
                    },
                    notification = ancs_rx.recv(), if ancs_task.is_some() => match notification {
                        Some(n) if n.is_message() => {
                            tracing::info!(uid = n.uid, "ANCS: new message, syncing soon");
                            ancs_sync_at.get_or_insert(Instant::now() + ANCS_SYNC_DELAY);
                        }
                        Some(n) => tracing::trace!(app = ?n.app_identifier, "ANCS: ignoring non-message"),
                        None => {
                            let stop = match ancs_task.take() {
                                Some(task) => task.await.ok(),
                                None => None,
                            };
                            match stop {
                                Some(Ok(AncsStop::DeviceDisconnected)) => {
                                    return Outcome::Lost("phone disconnected".into());
                                }
                                Some(Ok(AncsStop::Cancelled)) => return Outcome::Stop,
                                Some(Err(e)) => {
                                    tracing::warn!(error = %e, "ANCS stopped, falling back to polling");
                                }
                                None => tracing::warn!("ANCS task ended unexpectedly"),
                            }
                            self.emit(LinkEvent::State(LinkState::Connected { live_notifications: false }));
                        }
                    },
                }
            }
        }
        .await;

        ancs_cancel.cancel();
        if let Some(task) = ancs_task {
            let _ = task.await;
        }
        map.disconnect().await;
        outcome
    }

    /// Run one sync. `Err` means the session is gone and we should reconnect.
    async fn sync(&mut self, map: &MapClient) -> std::result::Result<(), String> {
        let result = MessageSyncService::sync_all(map, &self.pool).await;
        let initial = !self.initial_sync_done;
        // Only a sync that actually saw the inbox counts as the initial import; otherwise the
        // backlog would arrive later as "new" and flood the notifications.
        if result.inbox_listed {
            self.initial_sync_done = true;
        }
        if !result.new_messages.is_empty() {
            self.emit(LinkEvent::NewMessages {
                messages: result.new_messages.clone(),
                initial,
            });
        }
        if let Some(err) = result.errors.first() {
            self.emit(LinkEvent::SyncFailed(err.clone()));
            if let Err(e) = map.ping().await {
                return Err(format!("phone stopped responding: {e}"));
            }
        }
        Ok(())
    }
}

/// Make sure the phone is connected over Bluetooth and open MAP.
async fn connect_map(device: &BluetoothDevice) -> Result<MapClient> {
    let manager = DeviceManager::new().await?;
    if !manager.is_connected(&device.address).await.unwrap_or(false) {
        manager.connect_device(&device.address).await?;
        // Profiles come up a moment after the ACL link.
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let mut map = MapClient::new(device.address.clone());
    map.connect().await?;
    Ok(map)
}

/// Push an SMS through MAP and store it as sent.
pub async fn send_and_store(
    map: &MapClient,
    pool: &SqlitePool,
    recipient: &str,
    text: &str,
) -> Result<Message> {
    let text = text.trim_end();
    if text.trim().is_empty() {
        return Err(BtsmsError::InvalidFormat("message is empty".into()));
    }
    let number = normalize_for_sending(recipient)?;
    map.send_sms(&number, text).await?;
    let now = Utc::now();
    let new = NewMessage {
        uid: format!(
            "local_{}_{}",
            now.timestamp_nanos_opt().unwrap_or(0),
            number
        ),
        device_source: "local".into(),
        direction: MessageDirection::Outgoing,
        peer: normalize_address(&number),
        peer_raw: number,
        peer_name: None,
        body: text.to_string(),
        timestamp: now,
        read: true,
        message_type: "SMS".into(),
        text_complete: true,
    };
    db::insert_message(pool, &new)
        .await?
        .ok_or_else(|| BtsmsError::InvalidFormat("sent message was not stored".into()))
}

/// Download the phonebook over PBAP and replace the stored contacts of that phone.
pub async fn sync_contacts(device_address: &str, pool: &SqlitePool) -> Result<usize> {
    let mut pbap = PbapClient::new(device_address.to_string());
    pbap.connect().await?;
    let result = pbap.pull_all_contacts().await;
    pbap.disconnect().await;
    let vcards = result?;
    ContactManager::new(pool.clone())
        .sync_from_vcards(&vcards, device_address)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(reconnect_delay(0), Duration::from_secs(2));
        assert_eq!(reconnect_delay(1), Duration::from_secs(2));
        assert_eq!(reconnect_delay(2), Duration::from_secs(5));
        assert_eq!(reconnect_delay(6), Duration::from_secs(60));
        assert_eq!(reconnect_delay(1000), Duration::from_secs(60));
    }

    #[test]
    fn settings_from_config() {
        let config = crate::config::Config {
            message_polling_enabled: false,
            message_polling_interval: 42,
            ..Default::default()
        };
        let s = LinkSettings::from_config(&config);
        assert!(!s.polling_enabled);
        assert_eq!(s.poll_interval, Duration::from_secs(42));
    }

    #[tokio::test]
    async fn sending_while_disconnected_fails_fast() {
        // A link whose task has already finished: commands cannot be delivered.
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        drop(cmd_rx);
        let link = PhoneLink {
            device: BluetoothDevice {
                address: "AA:BB:CC:DD:EE:FF".into(),
                name: "Test".into(),
                path: String::new(),
                icon: None,
                class: None,
                paired: true,
                connected: false,
                trusted: true,
            },
            handle: LinkHandle {
                cmd_tx,
                settings_tx: Arc::new(
                    watch::channel(LinkSettings {
                        polling_enabled: true,
                        poll_interval: Duration::from_secs(60),
                    })
                    .0,
                ),
            },
            cancel: CancellationToken::new(),
            task: None,
        };
        assert!(matches!(
            link.send_sms("+48512345678", "hi").await,
            Err(BtsmsError::NotConnected)
        ));
    }
}
