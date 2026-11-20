//! Import messages from the phone (MAP) into the local database.

use crate::bluetooth::{MapClient, MapFolder, MapMessage};
use crate::contacts::normalize_address;
use crate::db::{self, Message, MessageDirection, NewMessage};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

/// How far apart a phone "sent" entry and a locally stored outgoing message may be to count
/// as the same message.
const OUTGOING_DEDUP_WINDOW_MINUTES: i64 = 10;

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncResult {
    pub inbox_imported: usize,
    pub sent_imported: usize,
    /// Messages the phone listed that were already stored.
    pub already_known: usize,
    /// Full-text downloads that failed (the truncated subject was stored instead and the
    /// download is retried on the next sync).
    pub download_failures: usize,
    /// Previously truncated messages whose full text was downloaded now.
    pub completed: usize,
    /// The inbox could be listed (false when the phone refused or went away).
    pub inbox_listed: bool,
    pub errors: Vec<String>,
    #[serde(skip)]
    pub new_messages: Vec<Message>,
}

impl SyncResult {
    pub fn imported(&self) -> usize {
        self.inbox_imported + self.sent_imported
    }
}

pub struct MessageSyncService;

impl MessageSyncService {
    /// Import inbox and sent messages. A missing/unsupported "sent" folder (iPhone keeps
    /// it empty) is not an error.
    pub async fn sync_all(map: &MapClient, pool: &SqlitePool) -> SyncResult {
        let mut result = SyncResult::default();
        Self::sync_folder(map, pool, MapFolder::Inbox, &mut result).await;
        Self::sync_folder(map, pool, MapFolder::Sent, &mut result).await;
        tracing::info!(
            inbox = result.inbox_imported,
            sent = result.sent_imported,
            known = result.already_known,
            download_failures = result.download_failures,
            errors = result.errors.len(),
            "message sync finished"
        );
        result
    }

    pub async fn sync_folder(
        map: &MapClient,
        pool: &SqlitePool,
        folder: MapFolder,
        result: &mut SyncResult,
    ) {
        let listing = match map.list_messages(folder).await {
            Ok(l) => {
                if folder == MapFolder::Inbox {
                    result.inbox_listed = true;
                }
                l
            }
            Err(e) => {
                if folder == MapFolder::Inbox {
                    tracing::warn!(%folder, error = %e, "listing failed");
                    result.errors.push(format!("{folder}: {e}"));
                } else {
                    tracing::debug!(%folder, error = %e, "listing failed (phone may not expose it)");
                }
                return;
            }
        };

        for entry in listing {
            match import_entry(map, pool, &entry).await {
                Ok(Imported::New(message)) => {
                    match folder {
                        MapFolder::Inbox => result.inbox_imported += 1,
                        _ => result.sent_imported += 1,
                    }
                    result.new_messages.push(message);
                }
                Ok(Imported::Known) => result.already_known += 1,
                Ok(Imported::Completed) => {
                    result.already_known += 1;
                    result.completed += 1;
                }
                Ok(Imported::NewWithTruncatedText(message)) => {
                    result.download_failures += 1;
                    match folder {
                        MapFolder::Inbox => result.inbox_imported += 1,
                        _ => result.sent_imported += 1,
                    }
                    result.new_messages.push(message);
                }
                Err(e) => {
                    tracing::warn!(handle = entry.handle, error = %e, "could not store message");
                    result.errors.push(format!("{}: {e}", entry.handle));
                }
            }
        }
    }

    /// Kept for callers that only want the inbox.
    pub async fn import_inbox(map: &MapClient, pool: &SqlitePool) -> Result<usize, String> {
        let mut result = SyncResult::default();
        Self::sync_folder(map, pool, MapFolder::Inbox, &mut result).await;
        match result.errors.first() {
            Some(e) if result.inbox_imported == 0 => Err(e.clone()),
            _ => Ok(result.inbox_imported),
        }
    }

    pub async fn import_sent(map: &MapClient, pool: &SqlitePool) -> usize {
        let mut result = SyncResult::default();
        Self::sync_folder(map, pool, MapFolder::Sent, &mut result).await;
        result.sent_imported
    }
}

enum Imported {
    New(Message),
    NewWithTruncatedText(Message),
    /// A stored preview got its full text.
    Completed,
    Known,
}

/// Stored for messages without any text (e.g. picture-only MMS) so they are not fetched again.
const NO_TEXT_PLACEHOLDER: &str = "(message without text)";

/// Stable id for a MAP message (same format as older builds, so nothing is re-imported).
pub fn message_uid(entry: &MapMessage) -> String {
    format!("map_{}_{}", entry.handle, entry.timestamp)
}

async fn import_entry(
    map: &MapClient,
    pool: &SqlitePool,
    entry: &MapMessage,
) -> crate::error::Result<Imported> {
    let uid = message_uid(entry);
    match db::message_state(pool, &uid).await? {
        Some((_, true)) => return Ok(Imported::Known),
        Some((id, false)) => {
            // Earlier download failed: try again.
            return Ok(match map.get_message(entry).await {
                Ok(full) if !full.body.is_empty() => {
                    db::complete_message_text(pool, id, &full.body).await?;
                    tracing::info!(
                        handle = entry.handle,
                        "completed previously truncated message"
                    );
                    Imported::Completed
                }
                Ok(_) => Imported::Known,
                Err(e) => {
                    tracing::debug!(handle = entry.handle, error = %e, "full text still unavailable");
                    Imported::Known
                }
            });
        }
        None => {}
    }

    let mut new = build_new_message(entry, map.device_address());
    if !entry.subject_is_complete() {
        match map.get_message(entry).await {
            Ok(full) => {
                if !full.body.is_empty() {
                    new.body = full.body;
                }
            }
            Err(e) => {
                tracing::warn!(
                    handle = entry.handle,
                    error = %e,
                    "full text download failed, storing the listing preview for now"
                );
                new.text_complete = false;
            }
        }
    }
    if new.body.trim().is_empty() {
        new.body = NO_TEXT_PLACEHOLDER.to_string();
    }

    if new.direction == MessageDirection::Outgoing
        && db::claim_local_outgoing(
            pool,
            &uid,
            &new.peer,
            &new.body,
            new.timestamp,
            chrono::Duration::minutes(OUTGOING_DEDUP_WINDOW_MINUTES),
        )
        .await?
    {
        tracing::debug!(handle = entry.handle, "sent message matched a local copy");
        return Ok(Imported::Known);
    }

    match db::insert_message(pool, &new).await? {
        Some(message) => {
            tracing::debug!(
                id = message.id,
                peer = message.peer,
                direction = %message.direction,
                body = %crate::logging::body_summary(&message.body),
                complete = new.text_complete,
                "stored message"
            );
            Ok(if new.text_complete {
                Imported::New(message)
            } else {
                Imported::NewWithTruncatedText(message)
            })
        }
        None => Ok(Imported::Known),
    }
}

/// Map a listing entry to a database row (text = subject until the full text is fetched).
pub fn build_new_message(entry: &MapMessage, device: &str) -> NewMessage {
    let outgoing = matches!(entry.folder, MapFolder::Sent | MapFolder::Outbox);
    let raw_peer = if outgoing {
        entry
            .recipient_address
            .clone()
            .or_else(|| entry.recipient.clone())
    } else {
        entry
            .sender_address
            .clone()
            .or_else(|| (!entry.sender.is_empty()).then(|| entry.sender.clone()))
    }
    .unwrap_or_default();
    let peer = normalize_address(&raw_peer);

    // The phone's display name is only useful when it is a name, not the number again.
    let peer_name = if outgoing {
        entry.recipient.clone()
    } else {
        Some(entry.sender.clone())
    }
    .map(|n| n.trim().to_string())
    .filter(|n| !n.is_empty() && *n != raw_peer && normalize_address(n) != peer);

    NewMessage {
        uid: message_uid(entry),
        device_source: device.to_string(),
        direction: if outgoing {
            MessageDirection::Outgoing
        } else {
            MessageDirection::Incoming
        },
        peer,
        peer_raw: raw_peer,
        peer_name,
        body: entry.subject.clone(),
        timestamp: parse_map_time(&entry.timestamp).unwrap_or_else(Utc::now),
        read: outgoing || entry.read,
        message_type: map_type_label(&entry.message_type).to_string(),
        text_complete: true,
    }
}

fn map_type_label(t: &str) -> &'static str {
    match t.to_ascii_lowercase().as_str() {
        "mms" => "MMS",
        "email" => "EMAIL",
        "im" => "IM",
        _ => "SMS",
    }
}

/// Parse a MAP timestamp (`20240115T143022`, optionally with `Z` or `±HHMM`). Without an
/// offset the phone's (= our) local time zone is assumed.
pub fn parse_map_time(ts: &str) -> Option<DateTime<Utc>> {
    let ts = ts.trim();
    let base = ts.get(..15)?;
    let naive = NaiveDateTime::parse_from_str(base, "%Y%m%dT%H%M%S").ok()?;
    let rest = &ts[15..];
    if rest.is_empty() {
        return Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|t| t.with_timezone(&Utc));
    }
    if rest == "Z" {
        return Some(Utc.from_utc_datetime(&naive));
    }
    let sign = match rest.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 4 {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = digits[2..].parse().ok()?;
    let offset = chrono::FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))?;
    offset
        .from_local_datetime(&naive)
        .single()
        .map(|t| t.with_timezone(&Utc))
}

/// RFC 3339 rendering of a MAP timestamp (kept for display/compat).
pub fn parse_map_timestamp(ts: &str) -> String {
    parse_map_time(ts)
        .map(|t| t.with_timezone(&Local).to_rfc3339())
        .unwrap_or_else(|| ts.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(folder: MapFolder) -> MapMessage {
        MapMessage {
            path: "/org/bluez/obex/client/session0/message77".into(),
            handle: "77".into(),
            folder,
            subject: "Hello".into(),
            timestamp: "20261120T124531".into(),
            sender: "Jan".into(),
            sender_address: Some("+48512345678".into()),
            recipient: None,
            recipient_address: None,
            message_type: "sms-gsm".into(),
            size: 5,
            read: false,
        }
    }

    #[test]
    fn uid_is_compatible_with_old_builds() {
        assert_eq!(
            message_uid(&entry(MapFolder::Inbox)),
            "map_77_20261120T124531"
        );
    }

    #[test]
    fn incoming_entry_maps_sender() {
        let m = build_new_message(&entry(MapFolder::Inbox), "AA");
        assert_eq!(m.direction, MessageDirection::Incoming);
        assert_eq!(m.peer, "+48512345678");
        assert_eq!(m.peer_name.as_deref(), Some("Jan"));
        assert_eq!(m.body, "Hello");
        assert!(!m.read);
        assert_eq!(m.device_source, "AA");
        assert_eq!(m.message_type, "SMS");
    }

    #[test]
    fn phone_name_equal_to_number_is_dropped() {
        let mut e = entry(MapFolder::Inbox);
        e.sender = "+48 512 345 678".into();
        assert_eq!(build_new_message(&e, "AA").peer_name, None);
    }

    #[test]
    fn sender_without_address_falls_back_to_name_field() {
        let mut e = entry(MapFolder::Inbox);
        e.sender_address = None;
        e.sender = "ING".into();
        let m = build_new_message(&e, "AA");
        assert_eq!(m.peer, "ING");
        assert_eq!(m.peer_name, None);
    }

    #[test]
    fn sent_entry_maps_recipient() {
        let mut e = entry(MapFolder::Sent);
        e.recipient_address = Some("+48600100200".into());
        e.recipient = Some("Anna".into());
        let m = build_new_message(&e, "AA");
        assert_eq!(m.direction, MessageDirection::Outgoing);
        assert_eq!(m.peer, "+48600100200");
        assert_eq!(m.peer_name.as_deref(), Some("Anna"));
        assert!(m.read);
    }

    #[test]
    fn map_time_with_offsets() {
        let t = parse_map_time("20240115T143022+0100").unwrap();
        assert_eq!(crate::db::format_db_time(t), "2024-01-15T13:30:22Z");
        let t = parse_map_time("20240115T143022-0500").unwrap();
        assert_eq!(crate::db::format_db_time(t), "2024-01-15T19:30:22Z");
        let t = parse_map_time("20240115T143022Z").unwrap();
        assert_eq!(crate::db::format_db_time(t), "2024-01-15T14:30:22Z");
    }

    #[test]
    fn map_time_without_offset_is_local() {
        let t = parse_map_time("20240115T143022").unwrap();
        let local = t.with_timezone(&Local);
        assert_eq!(local.format("%Y%m%dT%H%M%S").to_string(), "20240115T143022");
    }

    #[test]
    fn map_time_invalid() {
        assert_eq!(parse_map_time(""), None);
        assert_eq!(parse_map_time("2024"), None);
        assert_eq!(parse_map_time("invalid timestamp!"), None);
        assert_eq!(parse_map_time("20241315T143022"), None);
        assert_eq!(parse_map_time("20240115T143022+1"), None);
        // Non-ASCII must not panic on slicing
        assert_eq!(parse_map_time("2024011ąT143022"), None);
    }

    #[test]
    fn compat_rfc3339_rendering() {
        assert!(parse_map_timestamp("20240115T143022+0100").contains("2024-01-15T"));
        assert_eq!(parse_map_timestamp("invalid"), "invalid");
    }

    #[test]
    fn sync_result_totals() {
        let r = SyncResult {
            inbox_imported: 2,
            sent_imported: 3,
            ..Default::default()
        };
        assert_eq!(r.imported(), 5);
    }
}
