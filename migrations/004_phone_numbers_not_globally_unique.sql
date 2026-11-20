-- 002 made phone_normalized globally UNIQUE, so two contacts sharing a number (or one
-- contact listing a number twice) made the whole contact fail to import.
DROP INDEX IF EXISTS idx_phone_normalized;
CREATE INDEX idx_phone_normalized ON phone_numbers(phone_normalized);
CREATE UNIQUE INDEX idx_phone_contact_number ON phone_numbers(contact_id, phone_normalized);
