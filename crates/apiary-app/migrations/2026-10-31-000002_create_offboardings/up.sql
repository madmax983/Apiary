-- Where each hive's offboarding is. Control-plane state: it belongs in the
-- control database only.
--
-- It also lands, empty, in every hive's file, because autumn-web 0.9 applies
-- every app migration set to the control database AND to each fleet
-- database, with no way to mark a set as control-only. See
-- docs/adr/0002-one-migration-set.md, and the test that keeps hive copies of
-- this table empty.
CREATE TABLE offboardings (
    hive TEXT PRIMARY KEY NOT NULL,
    workflow_id TEXT NOT NULL,
    phase TEXT NOT NULL,
    detail TEXT NOT NULL,
    -- The verified backup, once there is one. Restore copies it back.
    backup TEXT,
    grace_until TIMESTAMP,
    updated_at TIMESTAMP NOT NULL
);
