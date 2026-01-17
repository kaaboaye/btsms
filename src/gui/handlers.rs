use crate::gui::conversation_row::{add_conversation_row, parse_map_timestamp};
use crate::gui::state::{SharedAppState, SharedUiState};
use btsms::contacts::normalize_e164;
use btsms::db;
use gtk4::glib;
use gtk4::prelude::*;

pub async fn load_conversations(pool: sqlx::SqlitePool, ui_state: SharedUiState) {
    match db::get_conversations(&pool).await {
        Ok(conversations) => {
            glib::idle_add_local_once(move || {
                let ui = ui_state.borrow();
                // Clear existing
                while let Some(child) = ui.conversation_list.first_child() {
                    ui.conversation_list.remove(&child);
                }
                // Add conversations
                for conv in conversations {
                    add_conversation_row(&ui.conversation_list, &conv);
                }
            });
        }
        Err(e) => {
            eprintln!("Failed to load conversations: {}", e);
        }
    }
}

pub async fn refresh_conversations(app_state: SharedAppState, ui_state: SharedUiState) {
    let state = app_state.lock().await;
    if let Some(pool) = &state.db_pool {
        let pool_clone = pool.clone();
        drop(state);
        load_conversations(pool_clone, ui_state).await;
    }
}

pub fn start_refresh_timer(app_state: SharedAppState, ui_state: SharedUiState) {
    glib::timeout_add_seconds_local(30, move || {
        let app_state_clone = app_state.clone();
        let ui_state_clone = ui_state.clone();

        glib::spawn_future_local(async move {
            refresh_conversations(app_state_clone, ui_state_clone).await;
        });

        glib::ControlFlow::Continue
    });
}

/// Polls for new messages from the phone via MAP and imports them.
/// Returns the number of new messages imported.
pub async fn poll_messages(app_state: SharedAppState, ui_state: SharedUiState) -> usize {
    let state = app_state.lock().await;

    let (map_client, db_pool) = match (&state.map_client, &state.db_pool) {
        (Some(map), Some(pool)) => (map, pool),
        _ => return 0,
    };

    let mut imported_count = 0;

    // Import inbox messages (incoming)
    match import_inbox_messages(map_client, db_pool).await {
        Ok(count) => imported_count += count,
        Err(e) => eprintln!("Poll inbox error: {}", e),
    }

    // Import sent messages (outgoing)
    imported_count += import_sent_messages(map_client, db_pool).await;

    drop(state);

    // Refresh conversation list if any new messages
    if imported_count > 0 {
        refresh_conversations(app_state, ui_state).await;
    }

    imported_count
}

/// Starts a timer that polls for new messages based on config settings.
/// Also performs an initial poll immediately on startup.
/// The polling interval and enable/disable state are read from the config.
pub fn start_message_poll_timer(app_state: SharedAppState, ui_state: SharedUiState) {
    // Initial poll on startup (only if polling is enabled)
    let app_state_initial = app_state.clone();
    let ui_state_initial = ui_state.clone();
    glib::spawn_future_local(async move {
        let is_enabled = {
            let state = app_state_initial.lock().await;
            state.config.message_polling_enabled
        };
        if is_enabled {
            let count = poll_messages(app_state_initial, ui_state_initial).await;
            if count > 0 {
                eprintln!("Initial poll: imported {} messages", count);
            }
        }
    });

    // Get initial polling interval from config
    let app_state_interval = app_state.clone();
    glib::spawn_future_local(async move {
        let interval = {
            let state = app_state_interval.lock().await;
            state.config.message_polling_interval
        };

        // Start the periodic polling timer
        schedule_next_poll(app_state, ui_state, interval);
    });
}

/// Schedules the next poll iteration. This function reads the current config
/// each time to respect any changes to polling settings.
fn schedule_next_poll(app_state: SharedAppState, ui_state: SharedUiState, interval_seconds: u32) {
    glib::timeout_add_seconds_local_once(interval_seconds, move || {
        let app_state_clone = app_state.clone();
        let ui_state_clone = ui_state.clone();

        glib::spawn_future_local(async move {
            // Read current config settings
            let (is_enabled, current_interval) = {
                let state = app_state_clone.lock().await;
                (
                    state.config.message_polling_enabled,
                    state.config.message_polling_interval,
                )
            };

            // Only poll if enabled
            if is_enabled {
                let count = poll_messages(app_state_clone.clone(), ui_state_clone.clone()).await;
                if count > 0 {
                    eprintln!("Poll: imported {} messages", count);
                }
            }

            // Schedule the next poll with the current interval from config
            schedule_next_poll(app_state_clone, ui_state_clone, current_interval);
        });
    });
}

pub async fn save_message_to_db(
    pool: &sqlx::SqlitePool,
    recipient: &str,
    message: &str,
    direction: &str,
) {
    let normalized_recipient = normalize_e164(recipient).unwrap_or_else(|_| recipient.to_string());

    let message_uid = format!(
        "{}_{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        &normalized_recipient
    );
    let now = chrono::Utc::now().to_rfc3339();

    if let Err(e) = sqlx::query(
        "INSERT INTO sms_messages (message_uid, device_source, sender_number, sender_normalized, recipient_number, recipient_normalized, message_body, direction, received_at, message_type)
         VALUES (?, 'local', 'me', 'me', ?, ?, ?, ?, ?, 'SMS')"
    )
    .bind(&message_uid)
    .bind(recipient)
    .bind(&normalized_recipient)
    .bind(message)
    .bind(direction)
    .bind(&now)
    .execute(pool)
    .await
    {
        eprintln!("Failed to save message to database: {}", e);
    }
}

pub async fn import_inbox_messages(
    map_client: &btsms::bluetooth::MapClient,
    db_pool: &sqlx::SqlitePool,
) -> Result<usize, String> {
    let mut imported_count = 0;

    match map_client.list_inbox_messages().await {
        Ok(messages) => {
            for msg in &messages {
                let body = match map_client.get_message_content(&msg.handle).await {
                    Ok(content) => content,
                    Err(_) => msg.subject.clone(),
                };

                if !body.is_empty() {
                    let message_uid = format!("map_{}_{}", msg.handle, msg.timestamp);
                    let timestamp = parse_map_timestamp(&msg.timestamp);

                    let sender_phone = msg
                        .sender_address
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&msg.sender);
                    let sender_name = if msg.sender_address.is_some() {
                        Some(msg.sender.as_str())
                    } else {
                        None
                    };

                    let result = sqlx::query(
                        "INSERT OR IGNORE INTO sms_messages
                         (message_uid, device_source, sender_number, sender_normalized, sender_name, message_body,
                          direction, received_at, message_type, read_status)
                         VALUES (?, 'phone', ?, ?, ?, ?, 'INCOMING', ?, 'SMS', ?)",
                    )
                    .bind(&message_uid)
                    .bind(sender_phone)
                    .bind(sender_phone)
                    .bind(sender_name)
                    .bind(&body)
                    .bind(&timestamp)
                    .bind(msg.read)
                    .execute(db_pool)
                    .await;

                    if result.is_ok() {
                        imported_count += 1;
                    }
                }
            }
            Ok(imported_count)
        }
        Err(e) => Err(format!("Inbox: {}", e)),
    }
}

pub async fn import_sent_messages(
    map_client: &btsms::bluetooth::MapClient,
    db_pool: &sqlx::SqlitePool,
) -> usize {
    let mut imported_count = 0;

    match map_client.list_sent_messages().await {
        Ok(messages) => {
            for msg in &messages {
                let body = match map_client.get_message_content(&msg.handle).await {
                    Ok(content) => content,
                    Err(_) => msg.subject.clone(),
                };

                if !body.is_empty() {
                    let message_uid = format!("map_{}_{}", msg.handle, msg.timestamp);
                    let timestamp = parse_map_timestamp(&msg.timestamp);
                    let recipient_phone = msg
                        .recipient_address
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .or(msg.recipient.as_deref())
                        .unwrap_or("");

                    let result = sqlx::query(
                        "INSERT OR IGNORE INTO sms_messages
                         (message_uid, device_source, sender_normalized, recipient_number, recipient_normalized,
                          message_body, direction, received_at, message_type, read_status)
                         VALUES (?, 'phone', 'me', ?, ?, ?, 'OUTGOING', ?, 'SMS', 1)",
                    )
                    .bind(&message_uid)
                    .bind(recipient_phone)
                    .bind(recipient_phone)
                    .bind(&body)
                    .bind(&timestamp)
                    .execute(db_pool)
                    .await;

                    if result.is_ok() {
                        imported_count += 1;
                    }
                }
            }
        }
        Err(e) => {
            eprintln!(
                "Sent folder not available (normal for many phones): {}",
                e
            );
        }
    }

    imported_count
}
