-- Contacts table
CREATE TABLE contacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    display_name TEXT NOT NULL,
    given_name TEXT,
    family_name TEXT,
    vcard_id TEXT UNIQUE NOT NULL,
    source TEXT NOT NULL,  -- 'iphone', 'android', 'local'
    last_modified DATETIME,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    synced_at DATETIME
);

CREATE INDEX idx_contacts_display_name ON contacts(display_name);
CREATE INDEX idx_contacts_source ON contacts(source);

-- Phone numbers with E.164 normalization
CREATE TABLE phone_numbers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    phone_original TEXT NOT NULL,
    phone_normalized TEXT NOT NULL,  -- E.164 format: +15551234567
    phone_type TEXT NOT NULL,        -- CELL, WORK, HOME, OTHER
    is_primary BOOLEAN DEFAULT FALSE,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX idx_phone_normalized ON phone_numbers(phone_normalized);
CREATE INDEX idx_phone_contact_id ON phone_numbers(contact_id);

-- Email addresses
CREATE TABLE email_addresses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    email_type TEXT NOT NULL,
    is_primary BOOLEAN DEFAULT FALSE
);

-- Sync state tracking
CREATE TABLE sync_state (
    id INTEGER PRIMARY KEY,
    device_source TEXT NOT NULL,  -- 'iphone' or 'android'
    last_sync_time DATETIME,
    total_contacts_synced INTEGER DEFAULT 0
);
