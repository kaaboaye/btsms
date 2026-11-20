//! SQLite storage: connection setup, migrations and message queries.

mod messages;

pub use messages::*;

use crate::config::Config;
use crate::error::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Connection, Row, SqlitePool};
use std::path::{Path, PathBuf};
use std::time::Duration;

enum Migration {
    Sql(&'static str),
    /// Data fix-up done in Rust (needs parsing/normalisation SQLite cannot do).
    NormalizeRows,
}

/// Ordered migrations. Never edit or reorder applied entries; append new ones.
const MIGRATIONS: &[(i64, Migration, &str)] = &[
    (
        1,
        Migration::Sql(include_str!("../../migrations/001_initial.sql")),
        "",
    ),
    (
        2,
        Migration::Sql(include_str!("../../migrations/002_contacts.sql")),
        "contacts",
    ),
    (
        3,
        Migration::Sql(include_str!("../../migrations/003_messages.sql")),
        "sms_messages",
    ),
    (
        4,
        Migration::Sql(include_str!(
            "../../migrations/004_phone_numbers_not_globally_unique.sql"
        )),
        "",
    ),
    (
        5,
        Migration::Sql(include_str!("../../migrations/005_conversation_peer.sql")),
        "",
    ),
    (6, Migration::NormalizeRows, ""),
    (
        7,
        Migration::Sql(include_str!("../../migrations/007_text_complete.sql")),
        "",
    ),
];

/// Timestamps are stored as UTC RFC 3339 with second precision (`2026-11-20T11:45:31Z`) so
/// that text order equals time order. Older builds mixed local offsets and nanoseconds.
pub fn format_db_time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Parse a stored timestamp (any RFC 3339 variant).
pub fn parse_db_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Open (creating if needed) and migrate the database at `path`.
pub async fn init_database(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(10))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    run_migrations(&pool).await?;
    tracing::info!(path = %path.display(), "database ready");
    Ok(pool)
}

/// Open the database configured in `config`. With `merge_legacy`, the legacy `messages.db`
/// that older GUI builds wrote to (the CLI used `database_path`, so the two had diverged) is
/// merged in and renamed. Pass `false` for ad-hoc databases (`btsms-cli --database`) so they
/// don't swallow the user's history.
pub async fn open_app_database(config: &Config, merge_legacy: bool) -> Result<SqlitePool> {
    let pool = init_database(&config.database_path).await?;
    let legacy = Config::legacy_database_path();
    if merge_legacy && legacy.exists() && !same_file(&legacy, &config.database_path) {
        match merge_legacy_database(&pool, &legacy).await {
            Ok(merged) => {
                let backup = retire_legacy_file(&legacy);
                tracing::info!(
                    merged,
                    from = %legacy.display(),
                    backup = ?backup,
                    "merged legacy database"
                );
            }
            Err(e) => tracing::error!(
                error = %e,
                path = %legacy.display(),
                "could not merge legacy database; leaving it in place"
            ),
        }
    }
    Ok(pool)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Rename `messages.db` (and its WAL files) out of the way so it is merged only once.
fn retire_legacy_file(path: &Path) -> Option<PathBuf> {
    let backup = path.with_extension("db.merged");
    std::fs::rename(path, &backup).ok()?;
    for suffix in ["-wal", "-shm"] {
        let mut side = path.as_os_str().to_owned();
        side.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(side));
    }
    Some(backup)
}

/// Copy messages from another btsms database. Returns the number of new rows.
pub async fn merge_legacy_database(pool: &SqlitePool, legacy: &Path) -> Result<u64> {
    let legacy_str = legacy
        .to_str()
        .ok_or_else(|| crate::error::BtsmsError::Parse("legacy path is not UTF-8".into()))?;
    let mut conn = pool.acquire().await?;
    sqlx::query("ATTACH DATABASE ? AS legacy")
        .bind(legacy_str)
        .execute(&mut *conn)
        .await?;
    let result = async {
        let has_table: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM legacy.sqlite_master WHERE type='table' AND name='sms_messages')",
        )
        .fetch_one(&mut *conn)
        .await?;
        if !has_table {
            return Ok(0);
        }
        let mut tx = conn.begin().await?;
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO main.sms_messages (
                message_uid, device_source, sender_number, sender_normalized, sender_name,
                recipient_number, recipient_normalized, message_body, received_at, sent_at,
                read_status, message_type, direction, created_at)
             SELECT message_uid, device_source, sender_number, sender_normalized, sender_name,
                recipient_number, recipient_normalized, message_body, received_at, sent_at,
                read_status, message_type, direction, created_at
             FROM legacy.sms_messages",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok::<u64, crate::error::BtsmsError>(inserted)
    }
    .await;
    let _ = sqlx::query("DETACH DATABASE legacy")
        .execute(&mut *conn)
        .await;
    drop(conn);
    let inserted = result?;
    // Also covers a previous merge whose normalisation failed half-way.
    let missing_peer: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sms_messages WHERE peer IS NULL)")
            .fetch_one(pool)
            .await?;
    if inserted > 0 || missing_peer {
        renormalize(pool).await?;
    }
    Ok(inserted)
}

async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(pool)
    .await?;

    for (version, migration, marker_table) in MIGRATIONS {
        let applied: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM schema_version WHERE version = ?)")
                .bind(version)
                .fetch_one(pool)
                .await?;
        if applied {
            continue;
        }

        // Very old databases created tables without recording the version.
        if !marker_table.is_empty() {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            )
            .bind(marker_table)
            .fetch_one(pool)
            .await?;
            if exists {
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(version)
                    .execute(pool)
                    .await?;
                continue;
            }
        }

        tracing::info!(version, "applying database migration");
        match migration {
            Migration::Sql(sql) => {
                let mut tx = pool.begin().await?;
                sqlx::raw_sql(*sql).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(version)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
            Migration::NormalizeRows => {
                renormalize(pool).await?;
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(version)
                    .execute(pool)
                    .await?;
            }
        }
    }
    Ok(())
}

/// Bring every row in line with the current rules: UTC timestamps, and `peer` rebuilt from
/// the *raw* address columns (older builds stored already-mangled "normalised" numbers, e.g.
/// `+512345678` for a Polish `512345678`). Contact numbers are re-normalised too. Idempotent;
/// also used when the phone number region changes.
pub async fn renormalize(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT id, received_at, peer, direction, sender_number, sender_normalized,
                recipient_number, recipient_normalized
         FROM sms_messages",
    )
    .fetch_all(pool)
    .await?;

    let mut tx = pool.begin().await?;
    let mut changed = 0u64;
    for row in rows {
        let id: i64 = row.get("id");
        let received_at: String = row.get("received_at");
        let peer: Option<String> = row.get("peer");
        let direction: String = row.get("direction");
        let non_empty = |col: &str| {
            row.get::<Option<String>, _>(col)
                .filter(|v| !v.trim().is_empty() && v != "me")
        };
        let raw_peer = if direction == "OUTGOING" {
            non_empty("recipient_number").or_else(|| non_empty("recipient_normalized"))
        } else {
            non_empty("sender_number").or_else(|| non_empty("sender_normalized"))
        };

        let new_time = parse_db_time(&received_at)
            .map(format_db_time)
            .unwrap_or_else(|| received_at.clone());
        let new_peer = raw_peer
            .map(|p| crate::contacts::normalize_address(&p))
            .or_else(|| peer.clone());

        if new_time != received_at || new_peer != peer {
            changed +=
                sqlx::query("UPDATE sms_messages SET received_at = ?, peer = ? WHERE id = ?")
                    .bind(&new_time)
                    .bind(&new_peer)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
        }
    }

    let phones: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, phone_original, phone_normalized FROM phone_numbers")
            .fetch_all(&mut *tx)
            .await?;
    for (id, original, normalized) in phones {
        let renormalized = crate::contacts::normalize_address(&original);
        if renormalized != normalized && !renormalized.is_empty() {
            // OR IGNORE: two numbers of one contact may now collide; keep the old value then.
            sqlx::query("UPDATE OR IGNORE phone_numbers SET phone_normalized = ? WHERE id = ?")
                .bind(&renormalized)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    tracing::info!(changed, "normalised message rows");
    Ok(())
}

/// Delete all messages and contacts (keeps the schema).
pub async fn clear_all(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    for table in [
        "DELETE FROM sms_messages",
        "DELETE FROM email_addresses",
        "DELETE FROM phone_numbers",
        "DELETE FROM contacts",
        "DELETE FROM sync_state",
    ] {
        sqlx::query(table).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    tracing::warn!("database cleared");
    Ok(())
}

#[cfg(test)]
pub(crate) async fn test_pool() -> (tempfile::TempDir, SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_database(&dir.path().join("test.db")).await.unwrap();
    (dir, pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_database_has_all_migrations() {
        let (_dir, pool) = test_pool().await;
        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM schema_version ORDER BY version")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(versions, vec![1, 2, 3, 4, 5, 6, 7]);
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.db");
        init_database(&path).await.unwrap().close().await;
        let pool = init_database(&path).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 7);
    }

    #[tokio::test]
    async fn path_with_special_characters_works() {
        // The old code built a sqlite:// URL by hand, which breaks on '?' and '#'.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("we?ird #name.db");
        let pool = init_database(&path).await.unwrap();
        assert!(pool.acquire().await.is_ok());
        assert!(path.exists());
    }

    /// Schema and rows as written by the old build (version 3, mixed timestamps).
    async fn create_v3_database(path: &Path) {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let mut conn = sqlx::SqliteConnection::connect_with(&opts).await.unwrap();
        sqlx::raw_sql(include_str!("../../migrations/002_contacts.sql"))
            .execute(&mut conn)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/003_messages.sql"))
            .execute(&mut conn)
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at DATETIME);
             INSERT INTO schema_version (version) VALUES (1), (2), (3);
             INSERT INTO sms_messages (message_uid, device_source, sender_number, sender_normalized,
                 message_body, received_at, message_type, direction)
             VALUES ('map_1_20260512T101803', 'phone', '+48512345678', '+48512345678', 'hi',
                 '2026-05-12T10:18:03+02:00', 'SMS', 'INCOMING');
             INSERT INTO sms_messages (message_uid, device_source, sender_number, sender_normalized,
                 recipient_number, recipient_normalized, message_body, received_at, message_type, direction)
             VALUES ('17_x', 'local', 'me', 'me', '512345678', '+512345678', 'yo',
                 '2026-05-12T08:43:03.632737041+00:00', 'SMS', 'OUTGOING');",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        conn.close().await.unwrap();
    }

    #[tokio::test]
    async fn upgrade_from_v3_normalises_rows() {
        // The old GUI stored "+512345678" for a typed Polish "512345678"; the peer must be
        // rebuilt from the raw number so both directions land in one thread.
        crate::contacts::phone_normalizer::set_default_region(Some("PL"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        create_v3_database(&path).await;

        let pool = init_database(&path).await.unwrap();
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT received_at, peer FROM sms_messages ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "2026-05-12T08:18:03Z".to_string(),
                    "+48512345678".to_string()
                ),
                (
                    "2026-05-12T08:43:03Z".to_string(),
                    "+48512345678".to_string()
                ),
            ]
        );
        let convs = get_conversations(&pool).await.unwrap();
        assert_eq!(convs.len(), 1, "incoming and outgoing share one thread");
    }

    #[tokio::test]
    async fn legacy_database_is_merged_once() {
        crate::contacts::phone_normalizer::set_default_region(Some("PL"));
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("messages.db");
        create_v3_database(&legacy).await;
        let pool = init_database(&dir.path().join("btsms.db")).await.unwrap();

        assert_eq!(merge_legacy_database(&pool, &legacy).await.unwrap(), 2);
        assert_eq!(merge_legacy_database(&pool, &legacy).await.unwrap(), 0);
        let peers: Vec<Option<String>> = sqlx::query_scalar("SELECT peer FROM sms_messages")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(peers.iter().all(|p| p.as_deref() == Some("+48512345678")));

        let backup = retire_legacy_file(&legacy).unwrap();
        assert!(!legacy.exists());
        assert!(backup.exists());
    }

    #[tokio::test]
    async fn merge_repairs_rows_left_without_peer() {
        let (_dir, pool) = test_pool().await;
        insert_message(&pool, &NewMessage::test_incoming("+48512345678", "x"))
            .await
            .unwrap();
        sqlx::query("UPDATE sms_messages SET peer = NULL")
            .execute(&pool)
            .await
            .unwrap();
        let empty = tempfile::tempdir().unwrap();
        let legacy = empty.path().join("messages.db");
        init_database(&legacy).await.unwrap().close().await;
        assert_eq!(merge_legacy_database(&pool, &legacy).await.unwrap(), 0);
        let peer: Option<String> = sqlx::query_scalar("SELECT peer FROM sms_messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(peer.as_deref(), Some("+48512345678"));
    }

    #[tokio::test]
    async fn clear_all_empties_tables() {
        let (_dir, pool) = test_pool().await;
        insert_message(&pool, &NewMessage::test_incoming("+48512345678", "x"))
            .await
            .unwrap();
        clear_all(&pool).await.unwrap();
        assert!(get_recent_messages(&pool, 10).await.unwrap().is_empty());
    }

    #[test]
    fn db_time_format_sorts_lexicographically() {
        let a = parse_db_time("2026-05-12T10:18:03+02:00").unwrap();
        let b = parse_db_time("2026-05-12T08:43:03.632737041+00:00").unwrap();
        assert!(a < b);
        assert!(format_db_time(a) < format_db_time(b));
        assert_eq!(format_db_time(a), "2026-05-12T08:18:03Z");
        assert_eq!(parse_db_time("garbage"), None);
    }
}
