-- 0 = only the listing preview was stored because downloading the full text failed;
-- such messages are downloaded again on the next sync.
ALTER TABLE sms_messages ADD COLUMN text_complete BOOLEAN NOT NULL DEFAULT 1;
