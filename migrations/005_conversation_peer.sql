-- The other party of a message (normalised), used to group conversations.
ALTER TABLE sms_messages ADD COLUMN peer TEXT;
UPDATE sms_messages SET peer = CASE
    WHEN direction = 'OUTGOING' THEN COALESCE(recipient_normalized, sender_normalized)
    ELSE sender_normalized
END;
CREATE INDEX idx_messages_peer ON sms_messages(peer, received_at);
