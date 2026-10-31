# Findings: autumn-web 0.9's SQLite fleet, from building Apiary

Notes for the framework and its docs site (which, on 2026-10-31, still
documents 0.8.0 and has no fleet guide). Each is reproducible from this repo.

## 1. `max_open` is soft under bursts — and the config doc does not say so

`DatabaseFleetConfig::max_open` reads "Most databases kept open at once. The
least recently used idle one is closed past this." In a config-built app the
fleet uses `DEFAULT_MIN_RESIDENCY` (2 s): a file used in the last two seconds
is never an LRU victim, and request-path opens never refuse. So under a burst
across many tenants the open count runs far past the cap.

Measured: the default storm (1,000 hives, 6,000 touches, 8 workers,
≈1,300 ops/s) peaked at **594 open against `max_open = 16`**, then fell back to
16 within ~12 s and to 0 after `idle_close_secs`. Nothing leaked.

Suggestions:
- Say it in the `max_open` doc and the fleet guide: "a soft ceiling; files
  used within the residency grace are not closed to make room."
- Consider exposing `min_residency` in `[database.fleet]` (it is settable only
  through `DatabaseFleet::builder`).
- Consider a warning in the guide about descriptors: open files × pool size ×
  3 (db, `-wal`, `-shm`). 594 files can exceed a default macOS `ulimit -n` of
  256.

## 2. No way to scope an app migration set

Every app set goes to control **and** every fleet file. See
`docs/adr/0002-one-migration-set.md`.

## 3. `restore_missing` on `FleetLifecycle` doubles as "restore from a local backup"

`DatabaseFleet::restore` is replication-only. But every request-path open of a
missing file asks the lifecycle hooks' `restore_missing` (under the path lock,
after stray sidecars are cleared), regardless of `[database.fleet]
restore_missing`. Apiary uses that to put a verified backup back: stage the
backup, open the key, and let the fleet identity-check and migrate the file.
Worth documenting as a supported pattern — or adding a
`DatabaseFleet::import(key, path)`.

## 4. Smaller papercuts

- The generated repository for a SQLite fleet is still named
  `Pg<Model>Repository`.
- `[session] secure` defaults to `true` in the dev profile too; Safari and most
  HTTP clients will not send a `Secure` cookie over plain-http localhost, so
  session-sourced tenancy silently 401s under `cargo run` until
  `autumn-dev.toml` sets `secure = false`.
- `CsrfToken` as a required extractor 500s when CSRF is disabled by config;
  `Option<CsrfToken>` works (it implements `OptionalFromRequestParts`), but
  examples use the required form.
- `use diesel::prelude::*` next to `#[autumn_web::model]` / `#[repository]`
  makes the generated code ambiguous between sync and async `RunQueryDsl`
  (E0034 inside the macro expansion). Import only the derives.
- `autumn-harvest-sqlite` and `autumn-web`'s `sqlite` feature resolve to one
  `libsqlite3-sys` (0.38.2) — no `links` clash. `SqliteRuntime` and its drive
  future are `Send`, so a single tokio task can own it.
