-- SMS messages table
CREATE TABLE sms_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_uid TEXT UNIQUE NOT NULL,
    device_source TEXT NOT NULL,     -- 'iphone' or 'android'
    sender_number TEXT NOT NULL,
    sender_normalized TEXT NOT NULL, -- E.164 format
    sender_name TEXT,                -- Resolved from contacts
    recipient_number TEXT,
    recipient_normalized TEXT,
    message_body TEXT,
    received_at DATETIME NOT NULL,
    sent_at DATETIME,
    read_status BOOLEAN DEFAULT FALSE,
    message_type TEXT NOT NULL,      -- SMS, MMS, etc.
    direction TEXT NOT NULL,         -- INCOMING, OUTGOING
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_messages_sender ON sms_messages(sender_normalized);
CREATE INDEX idx_messages_received ON sms_messages(received_at DESC);
CREATE INDEX idx_messages_unread ON sms_messages(read_status, received_at DESC);
CREATE INDEX idx_messages_direction ON sms_messages(direction);
