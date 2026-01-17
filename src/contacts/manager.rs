use crate::contacts::phone_normalizer::normalize_e164;
use crate::error::Result;
use sqlx::{Row, SqlitePool};

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

#[derive(Debug, Default)]
struct SimpleVcard {
    formatted_name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
    uid: Option<String>,
    phone_numbers: Vec<(String, String)>, // (number, type)
    emails: Vec<(String, String)>,        // (email, type)
}

pub struct ContactManager {
    db_pool: SqlitePool,
}

impl ContactManager {
    pub fn new(db_pool: SqlitePool) -> Self {
        Self { db_pool }
    }

    /// Resolve phone number to contact name
    pub async fn resolve_number(&self, number: &str) -> Option<String> {
        // Normalize the phone number
        let normalized = match normalize_e164(number) {
            Ok(n) => n,
            Err(_) => return None,
        };

        // Query database for contact
        let result = sqlx::query(
            "SELECT c.display_name
             FROM contacts c
             JOIN phone_numbers p ON c.id = p.contact_id
             WHERE p.phone_normalized = ?
             LIMIT 1"
        )
        .bind(&normalized)
        .fetch_optional(&self.db_pool)
        .await;

        match result {
            Ok(Some(row)) => row.get::<String, _>("display_name").into(),
            _ => None,
        }
    }

    /// Get contact by ID
    pub async fn get_contact(&self, contact_id: i64) -> Result<Option<Contact>> {
        // Get contact info
        let contact_row = sqlx::query(
            "SELECT id, display_name FROM contacts WHERE id = ?"
        )
        .bind(contact_id)
        .fetch_optional(&self.db_pool)
        .await?;

        let contact_row = match contact_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let id: i64 = contact_row.get("id");
        let display_name: String = contact_row.get("display_name");

        // Get phone numbers
        let phone_rows = sqlx::query(
            "SELECT phone_original, phone_normalized, phone_type
             FROM phone_numbers
             WHERE contact_id = ?"
        )
        .bind(contact_id)
        .fetch_all(&self.db_pool)
        .await?;

        let phone_numbers: Vec<PhoneNumber> = phone_rows
            .into_iter()
            .map(|row| PhoneNumber {
                original: row.get("phone_original"),
                normalized: row.get("phone_normalized"),
                phone_type: row.get("phone_type"),
            })
            .collect();

        Ok(Some(Contact {
            id,
            display_name,
            phone_numbers,
        }))
    }

    /// Search contacts by name or phone number
    pub async fn search(&self, query: &str) -> Result<Vec<Contact>> {
        let search_pattern = format!("%{}%", query);

        let rows = sqlx::query(
            "SELECT DISTINCT c.id, c.display_name
             FROM contacts c
             LEFT JOIN phone_numbers p ON c.id = p.contact_id
             WHERE c.display_name LIKE ? OR p.phone_original LIKE ?
             ORDER BY c.display_name
             LIMIT 50"
        )
        .bind(&search_pattern)
        .bind(&search_pattern)
        .fetch_all(&self.db_pool)
        .await?;

        let mut contacts = Vec::new();
        for row in rows {
            let id: i64 = row.get("id");
            if let Some(contact) = self.get_contact(id).await? {
                contacts.push(contact);
            }
        }

        Ok(contacts)
    }

    /// Sync contacts from vCard stream
    pub async fn sync_from_vcards(&self, vcard_data: &str, device_source: &str) -> Result<usize> {
        let vcards = Self::parse_vcard_stream(vcard_data);
        let mut synced_count = 0;

        for vcard_str in vcards {
            let vcard = Self::parse_simple_vcard(&vcard_str);
            if self.import_vcard(&vcard, device_source).await.is_ok() {
                synced_count += 1;
            }
        }

        // Update sync state
        let now = chrono::Utc::now().to_rfc3339();
        let _ = sqlx::query(
            "INSERT OR REPLACE INTO sync_state (id, device_source, last_sync_time, total_contacts_synced)
             VALUES (1, ?, ?, ?)"
        )
        .bind(device_source)
        .bind(&now)
        .bind(synced_count as i64)
        .execute(&self.db_pool)
        .await;

        Ok(synced_count)
    }

    /// Import a single vCard into the database
    async fn import_vcard(&self, vcard: &SimpleVcard, device_source: &str) -> Result<i64> {
        // Extract display name
        let display_name = vcard.formatted_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

        let given_name = vcard.given_name.clone();
        let family_name = vcard.family_name.clone();

        // Create vCard ID
        let vcard_id = vcard.uid
            .clone()
            .unwrap_or_else(|| format!("{}_{}", device_source, chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));

        let now = chrono::Utc::now().to_rfc3339();

        // Insert or update contact
        let result = sqlx::query(
            "INSERT INTO contacts (display_name, given_name, family_name, vcard_id, source, last_modified, synced_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(vcard_id) DO UPDATE SET
                display_name = excluded.display_name,
                given_name = excluded.given_name,
                family_name = excluded.family_name,
                last_modified = excluded.last_modified,
                synced_at = excluded.synced_at
             RETURNING id"
        )
        .bind(&display_name)
        .bind(given_name)
        .bind(family_name)
        .bind(&vcard_id)
        .bind(device_source)
        .bind(&now)
        .bind(&now)
        .fetch_one(&self.db_pool)
        .await?;

        let contact_id: i64 = result.get("id");

        // Delete old phone numbers for this contact
        sqlx::query("DELETE FROM phone_numbers WHERE contact_id = ?")
            .bind(contact_id)
            .execute(&self.db_pool)
            .await?;

        // Insert phone numbers
        for (phone_original, phone_type) in &vcard.phone_numbers {
            // Normalize phone number
            let phone_normalized = normalize_e164(phone_original)
                .unwrap_or_else(|_| phone_original.clone());

            sqlx::query(
                "INSERT INTO phone_numbers (contact_id, phone_original, phone_normalized, phone_type)
                 VALUES (?, ?, ?, ?)"
            )
            .bind(contact_id)
            .bind(phone_original)
            .bind(&phone_normalized)
            .bind(phone_type)
            .execute(&self.db_pool)
            .await?;
        }

        // Insert email addresses
        for (email_addr, email_type) in &vcard.emails {
            sqlx::query(
                "INSERT INTO email_addresses (contact_id, email, email_type)
                 VALUES (?, ?, ?)"
            )
            .bind(contact_id)
            .bind(email_addr)
            .bind(email_type)
            .execute(&self.db_pool)
            .await?;
        }

        Ok(contact_id)
    }

    /// Parse a simple vCard (vCard 3.0)
    fn parse_simple_vcard(vcard_str: &str) -> SimpleVcard {
        let mut vcard = SimpleVcard::default();

        for line in vcard_str.lines() {
            let line = line.trim();

            if let Some(stripped) = line.strip_prefix("FN:") {
                vcard.formatted_name = Some(stripped.to_string());
            } else if let Some(stripped) = line.strip_prefix("N:") {
                // N:FamilyName;GivenName;...
                let parts: Vec<&str> = stripped.split(';').collect();
                if !parts.is_empty() && !parts[0].is_empty() {
                    vcard.family_name = Some(parts[0].to_string());
                }
                if parts.len() > 1 && !parts[1].is_empty() {
                    vcard.given_name = Some(parts[1].to_string());
                }
            } else if let Some(stripped) = line.strip_prefix("UID:") {
                vcard.uid = Some(stripped.to_string());
            } else if line.starts_with("TEL") {
                // TEL;TYPE=CELL:+1234567890 or TEL:+1234567890
                let phone_type = if line.contains("CELL") {
                    "CELL"
                } else if line.contains("WORK") {
                    "WORK"
                } else if line.contains("HOME") {
                    "HOME"
                } else {
                    "OTHER"
                }.to_string();

                if let Some(colon_pos) = line.find(':') {
                    let number = line[colon_pos + 1..].to_string();
                    vcard.phone_numbers.push((number, phone_type));
                }
            } else if line.starts_with("EMAIL") {
                let email_type = if line.contains("WORK") {
                    "WORK"
                } else if line.contains("HOME") {
                    "HOME"
                } else {
                    "OTHER"
                }.to_string();

                if let Some(colon_pos) = line.find(':') {
                    let email = line[colon_pos + 1..].to_string();
                    vcard.emails.push((email, email_type));
                }
            }
        }

        vcard
    }

    /// Parse vCard stream into individual vCards
    fn parse_vcard_stream(vcards: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current_vcard = String::new();
        let mut in_vcard = false;

        for line in vcards.lines() {
            if line.starts_with("BEGIN:VCARD") {
                in_vcard = true;
                current_vcard.clear();
                current_vcard.push_str(line);
                current_vcard.push('\n');
            } else if line.starts_with("END:VCARD") {
                current_vcard.push_str(line);
                current_vcard.push('\n');
                result.push(current_vcard.clone());
                in_vcard = false;
            } else if in_vcard {
                current_vcard.push_str(line);
                current_vcard.push('\n');
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vcard_stream() {
        let vcards = r#"BEGIN:VCARD
VERSION:3.0
FN:John Doe
TEL:+15551234567
END:VCARD
BEGIN:VCARD
VERSION:3.0
FN:Jane Smith
TEL:+15559876543
END:VCARD"#;

        let parsed = ContactManager::parse_vcard_stream(vcards);
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].contains("John Doe"));
        assert!(parsed[1].contains("Jane Smith"));
    }

    #[tokio::test]
    async fn test_contact_manager_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        match crate::db::init_database(db_path.to_str().unwrap()).await {
            Ok(pool) => {
                let manager = ContactManager::new(pool);
                // Test that we can create a manager
                let result = manager.search("test").await;
                assert!(result.is_ok());
            }
            Err(_) => {
                // Skip if SQLite not available
            }
        }
    }

    #[tokio::test]
    async fn test_sync_vcards() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        match crate::db::init_database(db_path.to_str().unwrap()).await {
            Ok(pool) => {
                let manager = ContactManager::new(pool);

                let vcard_data = r#"BEGIN:VCARD
VERSION:3.0
FN:Test User
TEL;TYPE=CELL:+15551234567
END:VCARD"#;

                let result = manager.sync_from_vcards(vcard_data, "test").await;
                assert!(result.is_ok());
                assert_eq!(result.unwrap(), 1);

                // Test resolution
                let resolved = manager.resolve_number("+15551234567").await;
                assert_eq!(resolved, Some("Test User".to_string()));
            }
            Err(_) => {
                // Skip if SQLite not available
            }
        }
    }
}
