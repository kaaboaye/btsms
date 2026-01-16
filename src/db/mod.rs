use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use crate::error::Result;

pub mod schema;

pub async fn init_database(path: &str) -> Result<SqlitePool> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite:{}", path))
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    Ok(pool)
}

#[derive(Debug, Clone)]
pub struct Contact {
    pub id: i64,
    pub display_name: String,
    pub phone_numbers: Vec<PhoneNumber>,
}

#[derive(Debug, Clone)]
pub struct PhoneNumber {
    pub original: String,
    pub normalized: String,
    pub phone_type: String,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: i64,
    pub sender_number: String,
    pub sender_name: Option<String>,
    pub recipient_number: Option<String>,
    pub body: String,
    pub received_at: String,
    pub direction: MessageDirection,
    pub read_status: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDirection {
    Incoming,
    Outgoing,
}

impl std::fmt::Display for MessageDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incoming => write!(f, "INCOMING"),
            Self::Outgoing => write!(f, "OUTGOING"),
        }
    }
}

pub async fn get_recent_messages(pool: &SqlitePool, limit: i64) -> Result<Vec<Message>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, sender_number, sender_name, recipient_number, message_body,
               received_at, direction, read_status
        FROM sms_messages
        ORDER BY received_at DESC
        LIMIT ?
        "#,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|row| Message {
        id: row.id,
        sender_number: row.sender_number,
        sender_name: row.sender_name,
        recipient_number: row.recipient_number,
        body: row.message_body.unwrap_or_default(),
        received_at: row.received_at,
        direction: if row.direction == "INCOMING" {
            MessageDirection::Incoming
        } else {
            MessageDirection::Outgoing
        },
        read_status: row.read_status,
    }).collect())
}

pub async fn insert_message(
    pool: &SqlitePool,
    sender: &str,
    recipient: Option<&str>,
    body: &str,
    direction: MessageDirection,
) -> Result<i64> {
    let normalized_sender = crate::contacts::normalize_e164(sender).unwrap_or_else(|_| sender.to_string());
    let normalized_recipient = recipient.and_then(|r| crate::contacts::normalize_e164(r).ok());

    let now = chrono::Utc::now().to_rfc3339();
    let uid = format!("{}_{}", sender, now);

    let result = sqlx::query!(
        r#"
        INSERT INTO sms_messages (
            message_uid, device_source, sender_number, sender_normalized,
            recipient_number, recipient_normalized, message_body,
            received_at, message_type, direction
        ) VALUES (?, 'iphone', ?, ?, ?, ?, ?, ?, 'SMS', ?)
        "#,
        uid,
        sender,
        normalized_sender,
        recipient,
        normalized_recipient,
        body,
        now,
        direction.to_string()
    )
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_init() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let pool = init_database(db_path.to_str().unwrap()).await.unwrap();
        assert!(pool.acquire().await.is_ok());
    }

    #[tokio::test]
    async fn test_insert_and_retrieve_message() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let pool = init_database(db_path.to_str().unwrap()).await.unwrap();

        let id = insert_message(
            &pool,
            "+15551234567",
            Some("+15559876543"),
            "Test message",
            MessageDirection::Incoming
        ).await.unwrap();

        assert!(id > 0);

        let messages = get_recent_messages(&pool, 10).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, "Test message");
    }
}
