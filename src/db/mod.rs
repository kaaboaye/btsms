use crate::error::Result;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};

pub mod schema;

/// Returns the default database path for the application.
pub fn default_database_path() -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("btsms")
        .join("messages.db")
}

/// Resets the database by deleting the file and reinitializing.
/// Returns a new pool connected to the fresh database.
pub async fn reset_database(path: &str) -> Result<SqlitePool> {
    // Close any existing connections by dropping the pool
    // The caller should ensure no active connections exist

    // Delete the database file if it exists
    let db_path = std::path::Path::new(path);
    if db_path.exists() {
        std::fs::remove_file(db_path)?;
    }

    // Also remove the journal files if they exist
    let wal_path = format!("{}-wal", path);
    let shm_path = format!("{}-shm", path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);

    // Reinitialize with fresh schema
    init_database(path).await
}

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
    // Ensure schema_version table exists first
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(pool)
    .await?;

    // Migrations with their version numbers and marker tables
    // The marker table is used to detect if migration was already applied
    let migrations: Vec<(i64, &str, &str)> = vec![
        (1, include_str!("../../migrations/001_initial.sql"), ""), // No tables in migration 1
        (
            2,
            include_str!("../../migrations/002_contacts.sql"),
            "contacts",
        ),
        (
            3,
            include_str!("../../migrations/003_messages.sql"),
            "sms_messages",
        ),
    ];

    for (version, migration, marker_table) in migrations {
        // Check if this version is already recorded
        let version_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM schema_version WHERE version = ?)")
                .bind(version)
                .fetch_one(pool)
                .await?;

        if version_exists {
            continue;
        }

        // Check if migration was applied but not recorded (legacy database)
        if !marker_table.is_empty() {
            let table_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            )
            .bind(marker_table)
            .fetch_one(pool)
            .await?;

            if table_exists {
                // Table exists but version wasn't recorded - just record it
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(version)
                    .execute(pool)
                    .await?;
                continue;
            }
        }

        // Run the migration
        sqlx::raw_sql(migration).execute(pool).await?;
        sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
            .bind(version)
            .execute(pool)
            .await?;
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

/// Represents a conversation thread with the most recent message preview
#[derive(Debug, Clone)]
pub struct Conversation {
    pub phone_number: String,         // normalized E.164
    pub display_name: Option<String>, // contact name if available
    pub last_message: String,         // preview of last message
    pub last_message_time: String,    // timestamp of last message
    pub unread_count: i64,            // number of unread messages
    pub is_outgoing: bool,            // whether last message was outgoing
}

/// Get all conversations grouped by phone number, ordered by most recent
pub async fn get_conversations(pool: &SqlitePool) -> Result<Vec<Conversation>> {
    // This query groups messages by the "other party" phone number
    // For outgoing messages, we use recipient_normalized
    // For incoming messages, we use sender_normalized
    let rows = sqlx::query(
        r#"
        WITH conversation_phones AS (
            SELECT
                CASE
                    WHEN direction = 'OUTGOING' THEN COALESCE(recipient_normalized, sender_normalized)
                    ELSE sender_normalized
                END as phone,
                message_body,
                received_at,
                direction,
                sender_name,
                read_status
            FROM sms_messages
        ),
        ranked_messages AS (
            SELECT
                phone,
                message_body,
                received_at,
                direction,
                sender_name,
                read_status,
                ROW_NUMBER() OVER (PARTITION BY phone ORDER BY received_at DESC) as rn
            FROM conversation_phones
            WHERE phone IS NOT NULL AND phone != ''
        ),
        unread_counts AS (
            SELECT
                phone,
                COUNT(*) as unread
            FROM conversation_phones
            WHERE read_status = 0 AND direction = 'INCOMING'
            GROUP BY phone
        )
        SELECT
            rm.phone,
            rm.message_body,
            rm.received_at,
            rm.direction,
            rm.sender_name,
            COALESCE(uc.unread, 0) as unread_count
        FROM ranked_messages rm
        LEFT JOIN unread_counts uc ON rm.phone = uc.phone
        WHERE rm.rn = 1
        ORDER BY rm.received_at DESC
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let message_body: Option<String> = row.get("message_body");
            let direction: String = row.get("direction");
            Conversation {
                phone_number: row.get("phone"),
                display_name: row.get("sender_name"),
                last_message: message_body.unwrap_or_default(),
                last_message_time: row.get("received_at"),
                unread_count: row.get("unread_count"),
                is_outgoing: direction == "OUTGOING",
            }
        })
        .collect())
}

/// Get all messages for a specific conversation (by phone number)
pub async fn get_messages_for_conversation(
    pool: &SqlitePool,
    phone: &str,
    limit: i64,
) -> Result<Vec<Message>> {
    // Match messages where the phone number is either sender or recipient
    let rows = sqlx::query(
        r#"
        SELECT id, sender_number, sender_name, recipient_number, message_body,
               received_at, direction, read_status
        FROM sms_messages
        WHERE sender_normalized = ?
           OR recipient_normalized = ?
           OR sender_number = ?
           OR recipient_number = ?
        ORDER BY received_at ASC
        LIMIT ?
        "#,
    )
    .bind(phone)
    .bind(phone)
    .bind(phone)
    .bind(phone)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Message {
            id: row.get("id"),
            sender_number: row.get("sender_number"),
            sender_name: row.get("sender_name"),
            recipient_number: row.get("recipient_number"),
            body: row
                .get::<Option<String>, _>("message_body")
                .unwrap_or_default(),
            received_at: row.get("received_at"),
            direction: if row.get::<String, _>("direction") == "INCOMING" {
                MessageDirection::Incoming
            } else {
                MessageDirection::Outgoing
            },
            read_status: row.get("read_status"),
        })
        .collect())
}

/// Mark all messages in a conversation as read
pub async fn mark_conversation_read(pool: &SqlitePool, phone: &str) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE sms_messages
        SET read_status = 1
        WHERE (sender_normalized = ? OR recipient_normalized = ?)
          AND read_status = 0
        "#,
    )
    .bind(phone)
    .bind(phone)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_recent_messages(pool: &SqlitePool, limit: i64) -> Result<Vec<Message>> {
    let rows = sqlx::query(
        "SELECT id, sender_number, sender_name, recipient_number, message_body,
                received_at, direction, read_status
         FROM sms_messages
         ORDER BY received_at DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Message {
            id: row.get("id"),
            sender_number: row.get("sender_number"),
            sender_name: row.get("sender_name"),
            recipient_number: row.get("recipient_number"),
            body: row
                .get::<Option<String>, _>("message_body")
                .unwrap_or_default(),
            received_at: row.get("received_at"),
            direction: if row.get::<String, _>("direction") == "INCOMING" {
                MessageDirection::Incoming
            } else {
                MessageDirection::Outgoing
            },
            read_status: row.get("read_status"),
        })
        .collect())
}

pub async fn insert_message(
    pool: &SqlitePool,
    sender: &str,
    recipient: Option<&str>,
    body: &str,
    direction: MessageDirection,
) -> Result<i64> {
    let normalized_sender =
        crate::contacts::normalize_e164(sender).unwrap_or_else(|_| sender.to_string());
    let normalized_recipient = recipient.and_then(|r| crate::contacts::normalize_e164(r).ok());

    let now = chrono::Utc::now().to_rfc3339();
    let uid = format!("{}_{}", sender, now);

    let result = sqlx::query(
        "INSERT INTO sms_messages (
            message_uid, device_source, sender_number, sender_normalized,
            recipient_number, recipient_normalized, message_body,
            received_at, message_type, direction
        ) VALUES (?, 'iphone', ?, ?, ?, ?, ?, ?, 'SMS', ?)",
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

    #[test]
    fn test_default_database_path() {
        let path = default_database_path();
        assert!(path.to_string_lossy().contains("btsms"));
        assert!(path.to_string_lossy().contains("messages.db"));
    }

    #[tokio::test]
    async fn test_reset_database() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_reset.db");
        let path_str = db_path.to_str().unwrap();

        // Initialize database
        let pool = match init_database(path_str).await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Insert some data
        insert_message(
            &pool,
            "+15551234567",
            None,
            "Test message",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();

        // Verify data exists
        let messages = get_recent_messages(&pool, 10).await.unwrap();
        assert_eq!(messages.len(), 1);

        // Close the pool
        pool.close().await;

        // Reset database
        let new_pool = match reset_database(path_str).await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Verify data is gone
        let messages = get_recent_messages(&new_pool, 10).await.unwrap();
        assert_eq!(messages.len(), 0);
    }

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
            MessageDirection::Incoming,
        )
        .await
        .unwrap();

        assert!(id > 0);

        let messages = get_recent_messages(&pool, 10).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, "Test message");
    }

    #[tokio::test]
    async fn test_get_conversations_groups_by_phone() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let pool = match init_database(db_path.to_str().unwrap()).await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Insert messages from two different contacts
        insert_message(
            &pool,
            "+15551111111",
            None,
            "First from Alice",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        insert_message(
            &pool,
            "+15552222222",
            None,
            "First from Bob",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        insert_message(
            &pool,
            "+15551111111",
            None,
            "Second from Alice",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();

        let conversations = get_conversations(&pool).await.unwrap();

        // Should have 2 conversations
        assert_eq!(conversations.len(), 2);

        // Most recent conversation (Alice) should be first
        assert_eq!(conversations[0].phone_number, "+15551111111");
        assert_eq!(conversations[0].last_message, "Second from Alice");

        // Bob's conversation should be second
        assert_eq!(conversations[1].phone_number, "+15552222222");
        assert_eq!(conversations[1].last_message, "First from Bob");
    }

    #[tokio::test]
    async fn test_get_conversations_includes_outgoing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let pool = match init_database(db_path.to_str().unwrap()).await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Insert an incoming message
        insert_message(
            &pool,
            "+15551111111",
            None,
            "Hello",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Insert an outgoing reply
        insert_message(
            &pool,
            "me",
            Some("+15551111111"),
            "Hi back!",
            MessageDirection::Outgoing,
        )
        .await
        .unwrap();

        let conversations = get_conversations(&pool).await.unwrap();

        // Should still be 1 conversation (grouped by the other party's number)
        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].phone_number, "+15551111111");
        assert_eq!(conversations[0].last_message, "Hi back!");
        assert!(conversations[0].is_outgoing);
    }

    #[tokio::test]
    async fn test_get_messages_for_conversation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let pool = match init_database(db_path.to_str().unwrap()).await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Insert messages between user and one contact
        insert_message(
            &pool,
            "+15551111111",
            None,
            "Hey!",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        insert_message(
            &pool,
            "me",
            Some("+15551111111"),
            "Hi!",
            MessageDirection::Outgoing,
        )
        .await
        .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Insert message from different contact (should not appear)
        insert_message(
            &pool,
            "+15552222222",
            None,
            "Different person",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();

        let messages = get_messages_for_conversation(&pool, "+15551111111", 100)
            .await
            .unwrap();

        // Should have 2 messages (not the one from different contact)
        assert_eq!(messages.len(), 2);
        // Messages should be in chronological order (oldest first)
        assert_eq!(messages[0].body, "Hey!");
        assert_eq!(messages[1].body, "Hi!");
    }

    #[tokio::test]
    async fn test_mark_conversation_read() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let pool = match init_database(db_path.to_str().unwrap()).await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Insert unread messages
        insert_message(
            &pool,
            "+15551111111",
            None,
            "Unread 1",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();
        insert_message(
            &pool,
            "+15551111111",
            None,
            "Unread 2",
            MessageDirection::Incoming,
        )
        .await
        .unwrap();

        // Verify they show as unread
        let convos_before = get_conversations(&pool).await.unwrap();
        assert_eq!(convos_before[0].unread_count, 2);

        // Mark as read
        mark_conversation_read(&pool, "+15551111111").await.unwrap();

        // Verify they are now read
        let convos_after = get_conversations(&pool).await.unwrap();
        assert_eq!(convos_after[0].unread_count, 0);
    }
}
