-- A hive's notes. Applied to every hive's own SQLite file when the fleet
-- creates or opens it (ADR 0019 in autumn-web). Notes are append-only in
-- Apiary: there is no route that deletes one, which is what lets offboarding
-- check a backup by counting rows (docs/SPEC.md section 3.8).
--
-- `tenant_id` is redundant inside a per-tenant file, and is kept on purpose:
-- `#[repository(tenant_scoped)]` filters on it, so the same repository works
-- unchanged on a slot fleet, where several hives share one file.
CREATE TABLE notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    body TEXT NOT NULL,
    tenant_id TEXT,
    created_at TIMESTAMP NOT NULL
);
