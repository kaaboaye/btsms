use crate::gui::conversation_row::{add_conversation_row, parse_map_timestamp};
use crate::gui::state::{SharedAppState, SharedUiState};
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

pub async fn save_message_to_db(
    pool: &sqlx::SqlitePool,
    recipient: &str,
    message: &str,
    direction: &str,
) {
    let message_uid = format!(
        "{}_{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        recipient
    );
    let now = chrono::Utc::now().to_rfc3339();

    let _ = sqlx::query(
        "INSERT INTO sms_messages (message_uid, device_source, sender_normalized, recipient_normalized, message_body, direction, received_at, message_type)
         VALUES (?, 'local', 'me', ?, ?, ?, ?, 'SMS')"
    )
    .bind(&message_uid)
    .bind(recipient)
    .bind(message)
    .bind(direction)
    .bind(&now)
    .execute(pool)
    .await;
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
