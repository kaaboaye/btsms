//! Message and conversation queries.

use super::format_db_time;
use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MessageDirection {
    Incoming,
    Outgoing,
}

impl MessageDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "INCOMING",
            Self::Outgoing => "OUTGOING",
        }
    }
}

impl std::fmt::Display for MessageDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stored message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub id: i64,
    /// Normalised number (or address) of the other party; groups conversations.
    pub peer: String,
    pub sender_number: Option<String>,
    pub sender_name: Option<String>,
    pub recipient_number: Option<String>,
    pub body: String,
    /// UTC, RFC 3339.
    pub received_at: String,
    pub direction: MessageDirection,
    pub read_status: bool,
}

/// A message about to be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    /// Unique id; duplicates are ignored.
    pub uid: String,
    /// Phone address, or "local" for messages sent from this app.
    pub device_source: String,
    pub direction: MessageDirection,
    /// Normalised other party.
    pub peer: String,
    /// Other party as the phone reported it.
    pub peer_raw: String,
    /// Name the phone gave for the sender, if it is a real name.
    pub peer_name: Option<String>,
    pub body: String,
    pub timestamp: DateTime<Utc>,
    pub read: bool,
    /// "SMS", "MMS", "IM", ...
    pub message_type: String,
    /// False when only a truncated preview could be stored (retried on the next sync).
    pub text_complete: bool,
}

impl NewMessage {
    #[cfg(test)]
    pub fn test_incoming(peer: &str, body: &str) -> Self {
        Self {
            uid: format!(
                "test_{peer}_{body}_{}",
                Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            device_source: "test".into(),
            direction: MessageDirection::Incoming,
            peer: peer.into(),
            peer_raw: peer.into(),
            peer_name: None,
            body: body.into(),
            timestamp: Utc::now(),
            read: false,
            message_type: "SMS".into(),
            text_complete: true,
        }
    }
}

/// Insert a message. Returns the stored message, or `None` if its uid already existed.
pub async fn insert_message(pool: &SqlitePool, m: &NewMessage) -> Result<Option<Message>> {
    let (sender_number, sender_norm, sender_name, recipient_number, recipient_norm) =
        match m.direction {
            MessageDirection::Incoming => (
                m.peer_raw.as_str(),
                m.peer.as_str(),
                m.peer_name.as_deref(),
                None,
                None,
            ),
            MessageDirection::Outgoing => (
                "me",
                "me",
                None,
                Some(m.peer_raw.as_str()),
                Some(m.peer.as_str()),
            ),
        };
    let result = sqlx::query(
        "INSERT OR IGNORE INTO sms_messages (
            message_uid, device_source, sender_number, sender_normalized, sender_name,
            recipient_number, recipient_normalized, peer, message_body, received_at,
            read_status, message_type, direction, text_complete)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.uid)
    .bind(&m.device_source)
    .bind(sender_number)
    .bind(sender_norm)
    .bind(sender_name)
    .bind(recipient_number)
    .bind(recipient_norm)
    .bind(&m.peer)
    .bind(&m.body)
    .bind(format_db_time(m.timestamp))
    .bind(m.read)
    .bind(&m.message_type)
    .bind(m.direction.as_str())
    .bind(m.text_complete)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }
    get_message(pool, result.last_insert_rowid()).await
}

const MESSAGE_COLUMNS: &str = "id, COALESCE(peer, '') AS peer, sender_number, sender_name, \
     recipient_number, COALESCE(message_body, '') AS message_body, received_at, direction, \
     COALESCE(read_status, 0) AS read_status";

fn message_from_row(row: &SqliteRow) -> Message {
    Message {
        id: row.get("id"),
        peer: row.get("peer"),
        sender_number: row.get("sender_number"),
        sender_name: row.get("sender_name"),
        recipient_number: row.get("recipient_number"),
        body: row.get("message_body"),
        received_at: row.get("received_at"),
        direction: if row.get::<String, _>("direction") == "OUTGOING" {
            MessageDirection::Outgoing
        } else {
            MessageDirection::Incoming
        },
        read_status: row.get("read_status"),
    }
}

pub async fn get_message(pool: &SqlitePool, id: i64) -> Result<Option<Message>> {
    let sql = format!("SELECT {MESSAGE_COLUMNS} FROM sms_messages WHERE id = ?");
    Ok(sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .as_ref()
        .map(message_from_row))
}

pub async fn message_exists(pool: &SqlitePool, uid: &str) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sms_messages WHERE message_uid = ?)")
            .bind(uid)
            .fetch_one(pool)
            .await?,
    )
}

/// Stored state of a message: `(id, text_complete)`.
pub async fn message_state(pool: &SqlitePool, uid: &str) -> Result<Option<(i64, bool)>> {
    Ok(sqlx::query_as(
        "SELECT id, COALESCE(text_complete, 1) FROM sms_messages WHERE message_uid = ?",
    )
    .bind(uid)
    .fetch_optional(pool)
    .await?)
}

/// Replace a stored preview with the full text.
pub async fn complete_message_text(pool: &SqlitePool, id: i64, body: &str) -> Result<()> {
    sqlx::query("UPDATE sms_messages SET message_body = ?, text_complete = 1 WHERE id = ?")
        .bind(body)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// How far the timestamp of one MAP message may move between listings. The iPhone sometimes
/// lists a message again (same handle) one second earlier in a new session.
pub const MAP_RELIST_WINDOW_SECS: i64 = 120;

/// Handle of a message imported over MAP, from its uid `map_<handle>_<timestamp>`.
pub(crate) fn map_handle(uid: &str) -> Option<&str> {
    uid.strip_prefix("map_")?
        .rsplit_once('_')
        .map(|(handle, _)| handle)
        .filter(|h| !h.is_empty())
}

/// A stored MAP message with the same handle and peer whose timestamp is within
/// [`MAP_RELIST_WINDOW_SECS`] of `at`: the same message listed again with a shifted time.
pub async fn find_relisted(
    pool: &SqlitePool,
    handle: &str,
    peer: &str,
    at: DateTime<Utc>,
) -> Result<Option<i64>> {
    let escaped = handle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let window = chrono::Duration::seconds(MAP_RELIST_WINDOW_SECS);
    Ok(sqlx::query_scalar(
        "SELECT id FROM sms_messages
          WHERE message_uid LIKE ? ESCAPE '\\' AND peer = ?
            AND received_at BETWEEN ? AND ?
          ORDER BY id LIMIT 1",
    )
    .bind(format!("map\\_{escaped}\\_%"))
    .bind(peer)
    .bind(format_db_time(at - window))
    .bind(format_db_time(at + window))
    .fetch_optional(pool)
    .await?)
}

/// When the phone's "sent" folder lists a message this app sent itself, attach the phone's
/// uid to our local copy instead of storing it twice. Only unclaimed rows written by this app
/// (`device_source = 'local'`) are considered, so identical texts sent from the phone are
/// still stored. Returns true when a local row was claimed.
pub async fn claim_local_outgoing(
    pool: &SqlitePool,
    uid: &str,
    peer: &str,
    body: &str,
    at: DateTime<Utc>,
    window: chrono::Duration,
) -> Result<bool> {
    let claimed = sqlx::query(
        "UPDATE sms_messages SET message_uid = ?
          WHERE id = (
            SELECT id FROM sms_messages
             WHERE direction = 'OUTGOING' AND device_source = 'local'
               AND message_uid NOT LIKE 'map\\_%' ESCAPE '\\'
               AND peer = ? AND message_body = ?
               AND received_at BETWEEN ? AND ?
             ORDER BY received_at LIMIT 1)",
    )
    .bind(uid)
    .bind(peer)
    .bind(body)
    .bind(format_db_time(at - window))
    .bind(format_db_time(at + window))
    .execute(pool)
    .await?
    .rows_affected();
    Ok(claimed == 1)
}

/// Address to send to when replying in a conversation: the number exactly as the phone
/// reported it for an incoming message, else as last typed by the user. Never the
/// normalised key, which depends on the configured region.
pub async fn reply_address(pool: &SqlitePool, peer: &str) -> Result<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT CASE WHEN direction = 'INCOMING' THEN sender_number ELSE recipient_number END
           FROM sms_messages
          WHERE peer = ?
            AND COALESCE(CASE WHEN direction = 'INCOMING' THEN sender_number
                              ELSE recipient_number END, '') NOT IN ('', 'me')
          ORDER BY (direction = 'INCOMING') DESC, received_at DESC
          LIMIT 1",
    )
    .bind(peer)
    .fetch_optional(pool)
    .await?)
}

/// Newest `limit` messages overall, newest first.
pub async fn get_recent_messages(pool: &SqlitePool, limit: i64) -> Result<Vec<Message>> {
    let sql = format!(
        "SELECT {MESSAGE_COLUMNS} FROM sms_messages ORDER BY received_at DESC, id DESC LIMIT ?"
    );
    Ok(sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(limit)
        .fetch_all(pool)
        .await?
        .iter()
        .map(message_from_row)
        .collect())
}

/// The newest `limit` messages of a conversation, oldest first (display order).
pub async fn get_messages_for_conversation(
    pool: &SqlitePool,
    peer: &str,
    limit: i64,
) -> Result<Vec<Message>> {
    let sql = format!(
        "SELECT * FROM (
            SELECT {MESSAGE_COLUMNS} FROM sms_messages WHERE peer = ?
            ORDER BY received_at DESC, id DESC LIMIT ?
         ) ORDER BY received_at ASC, id ASC"
    );
    Ok(sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(peer)
        .bind(limit)
        .fetch_all(pool)
        .await?
        .iter()
        .map(message_from_row)
        .collect())
}

/// A conversation thread with its latest message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conversation {
    pub peer: String,
    /// Contact name, or the name the phone reported.
    pub display_name: Option<String>,
    pub last_message: String,
    pub last_message_time: String,
    pub unread_count: i64,
    pub is_outgoing: bool,
}

impl Conversation {
    pub fn title(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.peer)
    }
}

/// All conversations, most recent first.
pub async fn get_conversations(pool: &SqlitePool) -> Result<Vec<Conversation>> {
    let rows = sqlx::query(
        r#"
        WITH ranked AS (
            SELECT peer, message_body, received_at, direction,
                   ROW_NUMBER() OVER (PARTITION BY peer ORDER BY received_at DESC, id DESC) AS rn
            FROM sms_messages
            WHERE peer IS NOT NULL AND peer != '' AND peer != 'me'
        ),
        unread AS (
            SELECT peer, COUNT(*) AS n FROM sms_messages
            WHERE read_status = 0 AND direction = 'INCOMING'
            GROUP BY peer
        ),
        phone_names AS (
            SELECT peer, sender_name,
                   ROW_NUMBER() OVER (PARTITION BY peer ORDER BY received_at DESC, id DESC) AS rn
            FROM sms_messages
            WHERE sender_name IS NOT NULL AND sender_name != '' AND sender_name != peer
        )
        SELECT r.peer, COALESCE(r.message_body, '') AS message_body, r.received_at, r.direction,
               COALESCE(
                   (SELECT c.display_name FROM phone_numbers p
                      JOIN contacts c ON c.id = p.contact_id
                     WHERE p.phone_normalized = r.peer
                     ORDER BY c.display_name LIMIT 1),
                   n.sender_name
               ) AS display_name,
               COALESCE(u.n, 0) AS unread_count
        FROM ranked r
        LEFT JOIN unread u ON u.peer = r.peer
        LEFT JOIN phone_names n ON n.peer = r.peer AND n.rn = 1
        WHERE r.rn = 1
        ORDER BY r.received_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Conversation {
            peer: row.get("peer"),
            display_name: row.get("display_name"),
            last_message: row.get("message_body"),
            last_message_time: row.get("received_at"),
            unread_count: row.get("unread_count"),
            is_outgoing: row.get::<String, _>("direction") == "OUTGOING",
        })
        .collect())
}

/// Mark every message of a conversation as read locally.
pub async fn mark_conversation_read(pool: &SqlitePool, peer: &str) -> Result<u64> {
    Ok(
        sqlx::query("UPDATE sms_messages SET read_status = 1 WHERE peer = ? AND read_status = 0")
            .bind(peer)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// Total unread incoming messages.
pub async fn unread_count(pool: &SqlitePool) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM sms_messages WHERE read_status = 0 AND direction = 'INCOMING'",
    )
    .fetch_one(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::super::test_pool;
    use super::*;
    use chrono::Duration;

    fn msg(peer: &str, body: &str, dir: MessageDirection, minutes_ago: i64) -> NewMessage {
        NewMessage {
            direction: dir,
            timestamp: Utc::now() - Duration::minutes(minutes_ago),
            uid: format!("{peer}-{body}-{minutes_ago}"),
            ..NewMessage::test_incoming(peer, body)
        }
    }

    #[tokio::test]
    async fn insert_returns_message_and_ignores_duplicates() {
        let (_d, pool) = test_pool().await;
        let m = msg("+48512345678", "hi", MessageDirection::Incoming, 0);
        let stored = insert_message(&pool, &m).await.unwrap().unwrap();
        assert_eq!(stored.body, "hi");
        assert_eq!(stored.peer, "+48512345678");
        assert_eq!(stored.sender_number.as_deref(), Some("+48512345678"));
        assert!(stored.received_at.ends_with('Z'));
        assert!(insert_message(&pool, &m).await.unwrap().is_none());
        assert!(message_exists(&pool, &m.uid).await.unwrap());
        assert!(!message_exists(&pool, "nope").await.unwrap());
    }

    #[tokio::test]
    async fn conversations_group_by_peer_newest_first() {
        let (_d, pool) = test_pool().await;
        for m in [
            msg("+111", "First from Alice", MessageDirection::Incoming, 30),
            msg("+222", "First from Bob", MessageDirection::Incoming, 20),
            msg("+111", "Second from Alice", MessageDirection::Incoming, 10),
            msg("+222", "Reply to Bob", MessageDirection::Outgoing, 5),
        ] {
            insert_message(&pool, &m).await.unwrap();
        }
        let convs = get_conversations(&pool).await.unwrap();
        assert_eq!(convs.len(), 2);
        assert_eq!(convs[0].peer, "+222");
        assert_eq!(convs[0].last_message, "Reply to Bob");
        assert!(convs[0].is_outgoing);
        assert_eq!(convs[0].unread_count, 1);
        assert_eq!(convs[1].last_message, "Second from Alice");
        assert_eq!(convs[1].unread_count, 2);
    }

    #[tokio::test]
    async fn conversation_name_prefers_contacts_then_phone_name() {
        let (_d, pool) = test_pool().await;
        let mut m = msg("+111", "hi", MessageDirection::Incoming, 10);
        m.peer_name = Some("Alice (phone)".into());
        insert_message(&pool, &m).await.unwrap();
        // Last message outgoing: name must not disappear
        insert_message(&pool, &msg("+111", "yo", MessageDirection::Outgoing, 1))
            .await
            .unwrap();
        assert_eq!(
            get_conversations(&pool).await.unwrap()[0].title(),
            "Alice (phone)"
        );

        sqlx::raw_sql(
            "INSERT INTO contacts (id, display_name, vcard_id, source) VALUES (1, 'Alice Contact', 'x', 't');
             INSERT INTO phone_numbers (contact_id, phone_original, phone_normalized, phone_type)
             VALUES (1, '111', '+111', 'CELL');",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            get_conversations(&pool).await.unwrap()[0].title(),
            "Alice Contact"
        );
    }

    #[tokio::test]
    async fn conversation_messages_are_the_newest_in_display_order() {
        let (_d, pool) = test_pool().await;
        for i in 0..10 {
            insert_message(
                &pool,
                &msg(
                    "+111",
                    &format!("m{i}"),
                    MessageDirection::Incoming,
                    100 - i,
                ),
            )
            .await
            .unwrap();
        }
        insert_message(&pool, &msg("+222", "other", MessageDirection::Incoming, 0))
            .await
            .unwrap();
        let msgs = get_messages_for_conversation(&pool, "+111", 3)
            .await
            .unwrap();
        let bodies: Vec<_> = msgs.iter().map(|m| m.body.as_str()).collect();
        // The old query returned the *oldest* N messages.
        assert_eq!(bodies, vec!["m7", "m8", "m9"]);
    }

    #[tokio::test]
    async fn mark_read_and_unread_count() {
        let (_d, pool) = test_pool().await;
        insert_message(&pool, &msg("+111", "a", MessageDirection::Incoming, 2))
            .await
            .unwrap();
        insert_message(&pool, &msg("+111", "b", MessageDirection::Incoming, 1))
            .await
            .unwrap();
        insert_message(&pool, &msg("+222", "c", MessageDirection::Incoming, 1))
            .await
            .unwrap();
        assert_eq!(unread_count(&pool).await.unwrap(), 3);
        assert_eq!(mark_conversation_read(&pool, "+111").await.unwrap(), 2);
        assert_eq!(unread_count(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn local_outgoing_is_claimed_once() {
        let (_d, pool) = test_pool().await;
        let mut local = msg("+111", "on my way", MessageDirection::Outgoing, 0);
        local.device_source = "local".into();
        local.uid = "local_1".into();
        insert_message(&pool, &local).await.unwrap();
        let w = Duration::minutes(5);
        let at = local.timestamp;

        assert!(
            !claim_local_outgoing(&pool, "map_9_x", "+111", "other", at, w)
                .await
                .unwrap()
        );
        assert!(
            claim_local_outgoing(&pool, "map_9_x", "+111", "on my way", at, w)
                .await
                .unwrap()
        );
        assert!(message_exists(&pool, "map_9_x").await.unwrap());
        // Same text sent again from the phone: nothing left to claim, so it gets stored.
        assert!(
            !claim_local_outgoing(&pool, "map_10_x", "+111", "on my way", at, w)
                .await
                .unwrap()
        );
        assert!(
            !claim_local_outgoing(
                &pool,
                "map_11_x",
                "+111",
                "on my way",
                at + Duration::hours(1),
                w
            )
            .await
            .unwrap()
        );
    }

    #[test]
    fn map_handle_from_uid() {
        assert_eq!(map_handle("map_1048446_20261123T143307"), Some("1048446"));
        assert_eq!(map_handle("map_a_b_20261123T143307"), Some("a_b"));
        assert_eq!(map_handle("local_1"), None);
        assert_eq!(map_handle("map__20261123T143307"), None);
    }

    #[tokio::test]
    async fn relisted_message_is_found_by_handle_peer_and_time() {
        let (_d, pool) = test_pool().await;
        let mut m = msg("+111", "xD", MessageDirection::Incoming, 0);
        m.uid = "map_475_20261123T143309".into();
        let stored = insert_message(&pool, &m).await.unwrap().unwrap();
        let at = m.timestamp;
        let second = Duration::seconds(1);

        assert_eq!(
            find_relisted(&pool, "475", "+111", at - second)
                .await
                .unwrap(),
            Some(stored.id)
        );
        assert_eq!(find_relisted(&pool, "47", "+111", at).await.unwrap(), None);
        assert_eq!(find_relisted(&pool, "4_5", "+111", at).await.unwrap(), None);
        assert_eq!(find_relisted(&pool, "475", "+222", at).await.unwrap(), None);
        assert_eq!(
            find_relisted(&pool, "475", "+111", at + Duration::hours(1))
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn phone_sent_rows_are_not_claimed() {
        let (_d, pool) = test_pool().await;
        let phone = msg("+111", "ok", MessageDirection::Outgoing, 0); // device_source "test"
        insert_message(&pool, &phone).await.unwrap();
        assert!(
            !claim_local_outgoing(
                &pool,
                "map_1",
                "+111",
                "ok",
                phone.timestamp,
                Duration::minutes(5)
            )
            .await
            .unwrap()
        );
    }

    #[tokio::test]
    async fn truncated_text_can_be_completed() {
        let (_d, pool) = test_pool().await;
        let mut m = msg("+111", "preview…", MessageDirection::Incoming, 0);
        m.text_complete = false;
        let stored = insert_message(&pool, &m).await.unwrap().unwrap();
        assert_eq!(
            message_state(&pool, &m.uid).await.unwrap(),
            Some((stored.id, false))
        );
        complete_message_text(&pool, stored.id, "preview and the rest")
            .await
            .unwrap();
        assert_eq!(
            message_state(&pool, &m.uid).await.unwrap(),
            Some((stored.id, true))
        );
        assert_eq!(
            get_message(&pool, stored.id).await.unwrap().unwrap().body,
            "preview and the rest"
        );
        assert_eq!(message_state(&pool, "nope").await.unwrap(), None);
    }

    #[tokio::test]
    async fn reply_address_prefers_number_from_phone() {
        let (_d, pool) = test_pool().await;
        let mut out = msg("+48512345678", "hi", MessageDirection::Outgoing, 5);
        out.peer_raw = "512 345 678".into();
        insert_message(&pool, &out).await.unwrap();
        assert_eq!(
            reply_address(&pool, "+48512345678")
                .await
                .unwrap()
                .as_deref(),
            Some("512 345 678")
        );
        insert_message(
            &pool,
            &msg("+48512345678", "yo", MessageDirection::Incoming, 10),
        )
        .await
        .unwrap();
        assert_eq!(
            reply_address(&pool, "+48512345678")
                .await
                .unwrap()
                .as_deref(),
            Some("+48512345678")
        );
        assert_eq!(reply_address(&pool, "+1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn recent_messages_newest_first() {
        let (_d, pool) = test_pool().await;
        insert_message(&pool, &msg("+1", "old", MessageDirection::Incoming, 10))
            .await
            .unwrap();
        insert_message(&pool, &msg("+1", "new", MessageDirection::Incoming, 1))
            .await
            .unwrap();
        let msgs = get_recent_messages(&pool, 1).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "new");
    }
}
