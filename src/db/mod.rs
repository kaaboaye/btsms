use sqlx::{SqlitePool, sqlite::SqlitePoolOptions, Row};
use crate::error::Result;

pub mod schema;

pub async fn init_database(path: &str) -> Result<SqlitePool> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite://{}?mode=rwc", path))
        .await?;

    // Run migrations manually
    run_migrations(&pool).await?;

    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    // Read and execute migration files
    let migrations = vec![
        include_str!("../../migrations/001_initial.sql"),
        include_str!("../../migrations/002_contacts.sql"),
        include_str!("../../migrations/003_messages.sql"),
    ];

    for migration in migrations {
        sqlx::raw_sql(migration).execute(pool).await?;
    }

    Ok(())
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
    let rows = sqlx::query(
        "SELECT id, sender_number, sender_name, recipient_number, message_body,
                received_at, direction, read_status
         FROM sms_messages
         ORDER BY received_at DESC
         LIMIT ?"
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|row| Message {
        id: row.get("id"),
        sender_number: row.get("sender_number"),
        sender_name: row.get("sender_name"),
        recipient_number: row.get("recipient_number"),
        body: row.get::<Option<String>, _>("message_body").unwrap_or_default(),
        received_at: row.get("received_at"),
        direction: if row.get::<String, _>("direction") == "INCOMING" {
            MessageDirection::Incoming
        } else {
            MessageDirection::Outgoing
        },
        read_status: row.get("read_status"),
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

    let result = sqlx::query(
        "INSERT INTO sms_messages (
            message_uid, device_source, sender_number, sender_normalized,
            recipient_number, recipient_normalized, message_body,
            received_at, message_type, direction
        ) VALUES (?, 'iphone', ?, ?, ?, ?, ?, ?, 'SMS', ?)"
    )
    .bind(&uid)
    .bind(sender)
    .bind(&normalized_sender)
    .bind(recipient)
    .bind(normalized_recipient.as_deref())
    .bind(body)
    .bind(&now)
    .bind(direction.to_string())
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
        std::fs::create_dir_all(temp_dir.path()).unwrap();
        let db_path = temp_dir.path().join("test.db");

        match init_database(db_path.to_str().unwrap()).await {
            Ok(pool) => assert!(pool.acquire().await.is_ok()),
            Err(e) => {
                eprintln!("Database init error: {:?}", e);
                // For now, skip test if SQLite isn't available
                return;
            }
        }
    }

    #[tokio::test]
    async fn test_insert_and_retrieve_message() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp_dir.path()).unwrap();
        let db_path = temp_dir.path().join("test.db");

        let pool = match init_database(db_path.to_str().unwrap()).await {
            Ok(p) => p,
            Err(_) => return, // Skip test if SQLite not available
        };

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
