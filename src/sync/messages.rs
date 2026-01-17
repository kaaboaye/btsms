use crate::bluetooth::MapClient;
use sqlx::SqlitePool;

/// Result of a message sync operation
#[derive(Debug, Clone, Default)]
pub struct SyncResult {
    pub inbox_imported: usize,
    pub sent_imported: usize,
    pub errors: Vec<String>,
}

/// Service for syncing messages between phone and local database
pub struct MessageSyncService;

impl MessageSyncService {
    /// Sync all messages (inbox + sent) from the phone to the local database.
    /// Returns a SyncResult with counts and any errors encountered.
    pub async fn sync_all(map_client: &MapClient, db_pool: &SqlitePool) -> SyncResult {
        let mut result = SyncResult::default();

        // Import inbox messages (incoming)
        match Self::import_inbox(map_client, db_pool).await {
            Ok(count) => result.inbox_imported = count,
            Err(e) => result.errors.push(format!("Inbox: {}", e)),
        }

        // Import sent messages (outgoing)
        result.sent_imported = Self::import_sent(map_client, db_pool).await;

        result
    }

    /// Import inbox messages from phone to database.
    /// Returns the number of messages imported.
    pub async fn import_inbox(
        map_client: &MapClient,
        db_pool: &SqlitePool,
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
            Err(e) => Err(format!("{}", e)),
        }
    }

    /// Import sent messages from phone to database.
    /// Returns the number of messages imported.
    pub async fn import_sent(map_client: &MapClient, db_pool: &SqlitePool) -> usize {
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
}

/// Parse MAP timestamp format (e.g., "20240115T143022" or "20240115T143022+0100") to RFC3339
/// If no timezone is provided, assumes the timestamp is in local time.
pub fn parse_map_timestamp(timestamp: &str) -> String {
    if timestamp.len() >= 15 {
        let year = &timestamp[0..4];
        let month = &timestamp[4..6];
        let day = &timestamp[6..8];
        let hour = &timestamp[9..11];
        let minute = &timestamp[11..13];
        let second = &timestamp[13..15];

        // Check if timestamp has timezone info
        if timestamp.len() > 15 {
            let tz_part = &timestamp[15..];
            if (tz_part.starts_with('+') || tz_part.starts_with('-')) && tz_part.len() >= 5 {
                let tz = format!("{}:{}", &tz_part[..3], &tz_part[3..5]);
                return format!(
                    "{}-{}-{}T{}:{}:{}{}",
                    year, month, day, hour, minute, second, tz
                );
            }
        }

        // No timezone info - treat as local time
        // Parse as naive datetime, then convert to local timezone
        let naive_str = format!(
            "{}-{}-{}T{}:{}:{}",
            year, month, day, hour, minute, second
        );
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&naive_str, "%Y-%m-%dT%H:%M:%S") {
            if let Some(local_dt) = naive.and_local_timezone(chrono::Local).single() {
                return local_dt.to_rfc3339();
            }
        }
        // Fallback: return with local offset
        let local_offset = chrono::Local::now().offset().to_string();
        format!(
            "{}-{}-{}T{}:{}:{}{}",
            year, month, day, hour, minute, second, local_offset
        )
    } else if timestamp.is_empty() {
        chrono::Local::now().to_rfc3339()
    } else {
        timestamp.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_result_default() {
        let result = SyncResult::default();
        assert_eq!(result.inbox_imported, 0);
        assert_eq!(result.sent_imported, 0);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_sync_result_with_values() {
        let result = SyncResult {
            inbox_imported: 5,
            sent_imported: 3,
            errors: vec!["test error".to_string()],
        };
        assert_eq!(result.inbox_imported, 5);
        assert_eq!(result.sent_imported, 3);
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_parse_map_timestamp_basic() {
        // Timestamp without timezone should be treated as local time
        let result = parse_map_timestamp("20240115T143022");
        // Should produce a valid RFC3339 timestamp with the local timezone offset
        assert!(
            result.starts_with("2024-01-15T14:30:22"),
            "Expected timestamp to start with 2024-01-15T14:30:22, got: {}",
            result
        );
        // Should have a timezone offset (+ or -)
        assert!(
            result.contains('+') || result[19..].contains('-'),
            "Expected timezone offset in result: {}",
            result
        );
    }

    #[test]
    fn test_parse_map_timestamp_with_timezone() {
        let result = parse_map_timestamp("20240115T143022+0100");
        assert_eq!(result, "2024-01-15T14:30:22+01:00");
    }

    #[test]
    fn test_parse_map_timestamp_with_negative_timezone() {
        let result = parse_map_timestamp("20240115T143022-0500");
        assert_eq!(result, "2024-01-15T14:30:22-05:00");
    }

    #[test]
    fn test_parse_map_timestamp_empty() {
        let result = parse_map_timestamp("");
        // Should return current timestamp
        assert!(result.contains('T'));
        assert!(result.contains('-'));
    }

    #[test]
    fn test_parse_map_timestamp_invalid() {
        let result = parse_map_timestamp("invalid");
        assert_eq!(result, "invalid");
    }

    #[test]
    fn test_parse_map_timestamp_short() {
        let result = parse_map_timestamp("2024");
        assert_eq!(result, "2024");
    }
}
