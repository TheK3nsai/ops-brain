-- The sessions feature was removed previously, but the table and foreign key on handoffs remained.
-- Dropping them to clean up the schema.

ALTER TABLE handoffs DROP COLUMN IF EXISTS from_session_id;
DROP TABLE IF EXISTS sessions CASCADE;
