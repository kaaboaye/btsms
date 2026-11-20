//! Contacts stored from PBAP phonebook downloads.

use crate::contacts::phone_normalizer::normalize_address;
use crate::contacts::vcard::{self, VCard};
use crate::error::Result;
use serde::Serialize;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Contact {
    pub id: i64,
    pub display_name: String,
    pub phone_numbers: Vec<PhoneNumber>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhoneNumber {
    pub original: String,
    pub normalized: String,
    pub phone_type: String,
}

#[derive(Clone)]
pub struct ContactManager {
    db_pool: SqlitePool,
}

impl ContactManager {
    pub fn new(db_pool: SqlitePool) -> Self {
        Self { db_pool }
    }

    /// Contact name for a normalised peer number.
    pub async fn resolve_name(&self, peer: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT c.display_name FROM contacts c
               JOIN phone_numbers p ON c.id = p.contact_id
              WHERE p.phone_normalized = ?
              ORDER BY c.display_name LIMIT 1",
        )
        .bind(peer)
        .fetch_optional(&self.db_pool)
        .await?)
    }

    /// Convenience: normalise `number` and look up its contact name.
    pub async fn resolve_number(&self, number: &str) -> Option<String> {
        self.resolve_name(&normalize_address(number))
            .await
            .ok()
            .flatten()
    }

    pub async fn get_contact(&self, contact_id: i64) -> Result<Option<Contact>> {
        let Some(row) = sqlx::query("SELECT id, display_name FROM contacts WHERE id = ?")
            .bind(contact_id)
            .fetch_optional(&self.db_pool)
            .await?
        else {
            return Ok(None);
        };
        let phone_numbers = sqlx::query(
            "SELECT phone_original, phone_normalized, phone_type FROM phone_numbers
              WHERE contact_id = ? ORDER BY id",
        )
        .bind(contact_id)
        .fetch_all(&self.db_pool)
        .await?
        .into_iter()
        .map(|r| PhoneNumber {
            original: r.get("phone_original"),
            normalized: r.get("phone_normalized"),
            phone_type: r.get("phone_type"),
        })
        .collect();
        Ok(Some(Contact {
            id: row.get("id"),
            display_name: row.get("display_name"),
            phone_numbers,
        }))
    }

    /// Contacts whose name or number contains `query`.
    pub async fn search(&self, query: &str) -> Result<Vec<Contact>> {
        let pattern = format!("%{}%", escape_like(query.trim()));
        let digits: String = query.chars().filter(char::is_ascii_digit).collect();
        // NULL never matches, so short digit runs don't match every number.
        let digit_pattern = (digits.len() >= 3).then(|| format!("%{digits}%"));
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT c.id FROM contacts c
               LEFT JOIN phone_numbers p ON c.id = p.contact_id
              WHERE c.display_name LIKE ?1 ESCAPE '\\'
                 OR p.phone_original LIKE ?1 ESCAPE '\\'
                 OR p.phone_normalized LIKE ?2
              ORDER BY c.display_name COLLATE NOCASE
              LIMIT 50",
        )
        .bind(&pattern)
        .bind(&digit_pattern)
        .fetch_all(&self.db_pool)
        .await?;
        let mut contacts = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(c) = self.get_contact(id).await? {
                contacts.push(c);
            }
        }
        Ok(contacts)
    }

    /// All contacts sorted by name.
    pub async fn list(&self, limit: i64) -> Result<Vec<(i64, String)>> {
        Ok(sqlx::query_as(
            "SELECT id, display_name FROM contacts ORDER BY display_name COLLATE NOCASE LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.db_pool)
        .await?)
    }

    /// Best number for a contact with exactly this name (case-insensitive), preferring mobile.
    /// Returned as stored on the phone, so sending does not depend on the region setting.
    pub async fn number_for_name(&self, name: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT p.phone_original FROM contacts c
               JOIN phone_numbers p ON c.id = p.contact_id
              WHERE c.display_name = ? COLLATE NOCASE
              ORDER BY (p.phone_type = 'CELL') DESC, p.id
              LIMIT 1",
        )
        .bind(name.trim())
        .fetch_optional(&self.db_pool)
        .await?)
    }

    /// Replace every contact from `source` (the phone's address) with the given phonebook
    /// dump. PBAP always returns the whole phonebook, so a full replace is the simplest
    /// correct sync: deleted contacts disappear, nothing is duplicated.
    pub async fn sync_from_vcards(&self, vcard_data: &str, source: &str) -> Result<usize> {
        let cards = vcard::parse_vcards(vcard_data);
        self.replace_contacts(&cards, source).await
    }

    pub async fn replace_contacts(&self, cards: &[VCard], source: &str) -> Result<usize> {
        let mut tx = self.db_pool.begin().await?;
        sqlx::query("DELETE FROM contacts WHERE source = ?")
            .bind(source)
            .execute(&mut *tx)
            .await?;

        let now = chrono::Utc::now().to_rfc3339();
        let mut imported = 0usize;
        let mut skipped = 0usize;
        for (index, card) in cards.iter().enumerate() {
            let Some(display_name) = card
                .display_name()
                .or_else(|| card.first_phone().map(str::to_string))
            else {
                skipped += 1;
                continue;
            };
            if card.phones.is_empty() && card.emails.is_empty() {
                skipped += 1;
                continue;
            }
            // vcard_id is globally unique: scope it by source and position.
            let vcard_id = match &card.uid {
                Some(uid) => format!("{source}:{uid}"),
                None => format!("{source}:#{index}"),
            };
            let contact_id: i64 = sqlx::query_scalar(
                "INSERT INTO contacts (display_name, given_name, family_name, vcard_id, source,
                                       last_modified, synced_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(vcard_id) DO UPDATE SET display_name = excluded.display_name
                 RETURNING id",
            )
            .bind(&display_name)
            .bind(&card.given_name)
            .bind(&card.family_name)
            .bind(&vcard_id)
            .bind(source)
            .bind(&now)
            .bind(&now)
            .fetch_one(&mut *tx)
            .await?;

            for (i, phone) in card.phones.iter().enumerate() {
                let normalized = normalize_address(&phone.number);
                if normalized.is_empty() {
                    continue;
                }
                sqlx::query(
                    "INSERT OR IGNORE INTO phone_numbers
                        (contact_id, phone_original, phone_normalized, phone_type, is_primary)
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(contact_id)
                .bind(&phone.number)
                .bind(&normalized)
                .bind(&phone.kind)
                .bind(i == 0)
                .execute(&mut *tx)
                .await?;
            }
            for (i, email) in card.emails.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO email_addresses (contact_id, email, email_type, is_primary)
                     VALUES (?, ?, ?, ?)",
                )
                .bind(contact_id)
                .bind(&email.address)
                .bind(&email.kind)
                .bind(i == 0)
                .execute(&mut *tx)
                .await?;
            }
            imported += 1;
        }

        sqlx::query(
            "INSERT OR REPLACE INTO sync_state (id, device_source, last_sync_time, total_contacts_synced)
             VALUES (1, ?, ?, ?)",
        )
        .bind(source)
        .bind(&now)
        .bind(imported as i64)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        tracing::info!(imported, skipped, total = cards.len(), "contacts replaced");
        Ok(imported)
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    const BOOK: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Jan Kowalski\r\n\
        TEL;TYPE=CELL:+48 512 345 678\r\nTEL;TYPE=CELL:+48 512 345 678\r\n\
        EMAIL;TYPE=HOME:jan@example.com\r\nEND:VCARD\r\n\
        BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Anna Nowak\r\nTEL;TYPE=WORK:+48 22 123 45 67\r\n\
        TEL;TYPE=CELL:+48 600 100 200\r\nEND:VCARD\r\n\
        BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Shared Office\r\nTEL:+48 22 123 45 67\r\nEND:VCARD\r\n\
        BEGIN:VCARD\r\nVERSION:3.0\r\nFN:No Number\r\nEND:VCARD\r\n";

    #[tokio::test]
    async fn sync_imports_and_resolves() {
        let (_d, pool) = test_pool().await;
        let mgr = ContactManager::new(pool);
        assert_eq!(mgr.sync_from_vcards(BOOK, "AA:BB").await.unwrap(), 3);
        assert_eq!(
            mgr.resolve_name("+48512345678").await.unwrap().as_deref(),
            Some("Jan Kowalski")
        );
        assert_eq!(
            mgr.resolve_number("+48 600 100 200").await.as_deref(),
            Some("Anna Nowak")
        );
        assert_eq!(mgr.resolve_name("+49000").await.unwrap(), None);
    }

    #[tokio::test]
    async fn resync_replaces_instead_of_failing_or_duplicating() {
        // Old code: second sync generated new vcard_ids and hit the global UNIQUE on numbers.
        let (_d, pool) = test_pool().await;
        let mgr = ContactManager::new(pool.clone());
        mgr.sync_from_vcards(BOOK, "AA:BB").await.unwrap();
        assert_eq!(mgr.sync_from_vcards(BOOK, "AA:BB").await.unwrap(), 3);
        assert_eq!(mgr.list(100).await.unwrap().len(), 3);
        let emails: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM email_addresses")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(emails, 1, "emails are not duplicated");

        // Contact removed on the phone disappears
        let smaller = BOOK
            .split("BEGIN:VCARD")
            .take(2)
            .collect::<Vec<_>>()
            .join("BEGIN:VCARD");
        assert_eq!(mgr.sync_from_vcards(&smaller, "AA:BB").await.unwrap(), 1);
        assert_eq!(mgr.list(100).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn other_phones_contacts_are_kept() {
        let (_d, pool) = test_pool().await;
        let mgr = ContactManager::new(pool);
        mgr.sync_from_vcards(BOOK, "phone-a").await.unwrap();
        mgr.sync_from_vcards(BOOK, "phone-b").await.unwrap();
        assert_eq!(mgr.list(100).await.unwrap().len(), 6);
        mgr.sync_from_vcards("", "phone-a").await.unwrap();
        assert_eq!(mgr.list(100).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn search_by_name_and_number() {
        let (_d, pool) = test_pool().await;
        let mgr = ContactManager::new(pool);
        mgr.sync_from_vcards(BOOK, "x").await.unwrap();
        let by_name = mgr.search("kowal").await.unwrap();
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].phone_numbers.len(), 1, "duplicate TEL collapsed");
        let by_number = mgr.search("600 100").await.unwrap();
        assert_eq!(by_number[0].display_name, "Anna Nowak");
        assert!(
            mgr.search("%").await.unwrap().is_empty(),
            "LIKE wildcards are escaped"
        );
        assert!(mgr.search("_").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn number_for_name_prefers_mobile() {
        let (_d, pool) = test_pool().await;
        let mgr = ContactManager::new(pool);
        mgr.sync_from_vcards(BOOK, "x").await.unwrap();
        assert_eq!(
            mgr.number_for_name("anna nowak").await.unwrap().as_deref(),
            Some("+48 600 100 200")
        );
        assert_eq!(mgr.number_for_name("Nobody").await.unwrap(), None);
    }

    #[test]
    fn like_escaping() {
        assert_eq!(escape_like("50%_a\\b"), "50\\%\\_a\\\\b");
    }
}
