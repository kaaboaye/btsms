use crate::gui::message_bubble::{add_message_bubble, scroll_to_bottom};
use crate::gui::state::{SharedAppState, SharedUiState};
use btsms::bluetooth::{AncsClient, BluetoothDevice, DeviceManager, MapClient};
use btsms::config::Config;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Button, Label};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::handlers::{refresh_conversations, start_message_poll_timer, start_refresh_timer};

/// Result of auto-connect attempt
pub enum AutoConnectResult {
    /// Successfully found a device to connect to
    Device(BluetoothDevice),
    /// Multiple devices available, user must choose
    MultipleDevices,
    /// No devices available
    NoDevices,
    /// Error occurred
    Error(String),
}

/// Result of attempting to connect to a device
pub enum ConnectResult {
    Success { name: String },
    Failed(String),
}

/// Determine which device to auto-connect to based on:
/// 1. Last used device (if available and still paired)
/// 2. Single connected device
/// 3. Single paired device
/// 4. Multiple devices - ask user
pub async fn determine_auto_connect_device(config: &Config) -> AutoConnectResult {
    let manager = match DeviceManager::new().await {
        Ok(m) => m,
        Err(e) => return AutoConnectResult::Error(format!("Device manager error: {}", e)),
    };

    let phones = match manager.get_all_paired_phones().await {
        Ok(p) => p,
        Err(e) => return AutoConnectResult::Error(format!("Failed to get devices: {}", e)),
    };

    if phones.is_empty() {
        return AutoConnectResult::NoDevices;
    }

    // Check if last used device is still available
    if let Some(last_addr) = &config.last_device_address {
        if let Some(device) = phones.iter().find(|p| &p.address == last_addr) {
            eprintln!(
                "Auto-connect: Found last used device: {} ({})",
                device.name, device.address
            );
            return AutoConnectResult::Device(device.clone());
        }
    }

    // If only one device, use it
    if phones.len() == 1 {
        let device = phones.into_iter().next().unwrap();
        eprintln!(
            "Auto-connect: Single device available: {} ({})",
            device.name, device.address
        );
        return AutoConnectResult::Device(device);
    }

    // Check if there's exactly one connected device
    let connected: Vec<_> = phones.iter().filter(|p| p.connected).cloned().collect();
    if connected.len() == 1 {
        let device = connected.into_iter().next().unwrap();
        eprintln!(
            "Auto-connect: Single connected device: {} ({})",
            device.name, device.address
        );
        return AutoConnectResult::Device(device);
    }

    // Multiple devices - user must choose
    eprintln!("Auto-connect: Multiple devices available, user must choose");
    AutoConnectResult::MultipleDevices
}

/// Shared connection logic - connects to a device and sets up MAP
pub async fn connect_to_device(
    device: BluetoothDevice,
    app_state: Arc<Mutex<crate::gui::state::AppState>>,
    status_label: &Label,
) -> ConnectResult {
    let manager = match DeviceManager::new().await {
        Ok(m) => m,
        Err(e) => return ConnectResult::Failed(format!("Device manager error: {}", e)),
    };

    // Connect to Bluetooth if not already connected
    if !device.connected {
        status_label.set_text(&format!("Connecting to {}...", device.name));
        if let Err(e) = manager.connect_device(&device.address).await {
            return ConnectResult::Failed(format!("Failed to connect: {}", e));
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    // Connect to MAP
    status_label.set_text("Connecting to MAP...");
    let mut map_client = MapClient::new(device.address.clone());
    match map_client.connect().await {
        Ok(_) => {
            eprintln!("MAP connection successful to {}", device.name);
            let mut state_lock = app_state.lock().await;
            state_lock.map_client = Some(map_client);
            state_lock.device_address = Some(device.address.clone());
            state_lock.device_name = Some(device.name.clone());

            // Save as last used device
            state_lock.config.set_last_device(&device.address, &device.name);

            ConnectResult::Success { name: device.name }
        }
        Err(e) => ConnectResult::Failed(format!("{}", e)),
    }
}

pub async fn start_ancs_listener(
    app_state: SharedAppState,
    ui_state: SharedUiState,
    status_label: Label,
) {
    let app_state_clone = app_state.clone();

    glib::MainContext::default().spawn_local(async move {
        let mut ancs_client = AncsClient::new();

        match ancs_client.connect().await {
            Ok(_) => {
                eprintln!("ANCS connected - listening for notifications");
                status_label.set_text("Connected (ANCS active)");

                if let Some(mut rx) = ancs_client.take_notification_receiver() {
                    while let Some(notification) = rx.recv().await {
                        if let (Some(title), Some(message)) =
                            (&notification.title, &notification.message)
                        {
                            let sender = title.clone();
                            let msg = message.clone();

                            eprintln!("Received SMS: {} - {}", sender, msg);

                            // Add to UI if it's the current conversation
                            {
                                let state = app_state_clone.lock().await;
                                if state.current_conversation.as_deref() == Some(&sender) {
                                    let ui = ui_state.borrow();
                                    add_message_bubble(
                                        &ui.message_list,
                                        &msg,
                                        false,
                                        &chrono::Local::now().format("%H:%M").to_string(),
                                    );
                                    scroll_to_bottom(&ui.message_scroll);
                                }

                                // Save to database
                                if let Some(pool) = &state.db_pool {
                                    let message_uid = format!(
                                        "{}_{}",
                                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                                        sender
                                    );
                                    let now = chrono::Utc::now().to_rfc3339();

                                    let _ = sqlx::query(
                                        "INSERT INTO sms_messages (message_uid, device_source, sender_normalized, message_body, direction, received_at, message_type)
                                         VALUES (?, 'iphone', ?, ?, 'INCOMING', ?, 'SMS')"
                                    )
                                    .bind(&message_uid)
                                    .bind(&sender)
                                    .bind(&msg)
                                    .bind(&now)
                                    .execute(pool)
                                    .await;
                                }
                            }

                            // Refresh conversation list
                            refresh_conversations(app_state_clone.clone(), ui_state.clone()).await;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("ANCS not available (normal for Android): {}", e);
                status_label.set_text("Connected (MAP only)");
            }
        }
    });
}

pub async fn check_obexd_service() -> Result<bool, Box<dyn std::error::Error>> {
    let connection = zbus::Connection::session().await?;
    let proxy = zbus::fdo::DBusProxy::new(&connection).await?;
    let names = proxy.list_names().await?;
    Ok(names.iter().any(|name| name.as_str() == "org.bluez.obex"))
}

/// Completes the setup after a successful device connection.
/// This includes starting ANCS listener, refresh timer, and message polling.
/// Extracted to avoid code duplication across auto-connect, device switch, and manual connect.
pub async fn complete_connection_setup(
    app_state: SharedAppState,
    ui_state: SharedUiState,
    send_btn: &Button,
    device_switch: &Button,
    status: &Label,
    device_name: &str,
) {
    send_btn.set_sensitive(true);
    device_switch.set_visible(true);

    status.set_text("Connecting to ANCS...");
    start_ancs_listener(app_state.clone(), ui_state.clone(), status.clone()).await;
    start_refresh_timer(app_state.clone(), ui_state.clone());
    start_message_poll_timer(app_state, ui_state);

    status.set_text(&format!("Connected to {}", device_name));
}

/// Disconnects from the current device and updates UI state.
pub async fn disconnect_device(
    app_state: SharedAppState,
    status: &Label,
    device_switch: &Button,
    send_btn: &Button,
) {
    let mut state_lock = app_state.lock().await;
    if let Some(mut map_client) = state_lock.map_client.take() {
        let _ = map_client.disconnect().await;
    }
    state_lock.device_address = None;
    state_lock.device_name = None;
    drop(state_lock);

    status.set_text("Disconnected");
    device_switch.set_visible(false);
    send_btn.set_sensitive(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_connect_result_variants() {
        // Just verify the enum variants exist and can be constructed
        let _device = AutoConnectResult::NoDevices;
        let _multiple = AutoConnectResult::MultipleDevices;
        let _error = AutoConnectResult::Error("test".to_string());
    }

    #[test]
    fn test_connect_result_variants() {
        let _success = ConnectResult::Success {
            name: "Test".to_string(),
        };
        let _failed = ConnectResult::Failed("error".to_string());
    }
}
