# CLAUDE.md — Apiary

A test bed for autumn-web 0.9's SQLite database fleet. Read `PITCH.md`, then
`docs/SPEC.md`.

## Shape

- `crates/hive` — pure engine. No IO, async or clock. Anything with a rule
  belongs here, with property tests.
- `crates/apiary-app` — HTTP (Maud + htmx), the `DatabaseFleet`, the control
  database, and the `autumn-harvest-sqlite` runtime (owned by one tokio task,
  `offboard::Driver`).
- `verus/src/*.rs` — standalone Verus models of `hive::offboard` and
  `hive::storm::expand`. **Change the runtime and the model together**; the
  arms are listed side by side in SPEC §3.1.

## Rules that matter here

- The only irreversible action is deleting a hive file. It happens only in
  `offboard::purge_hive`, scheduled only behind `Offboarding::may_purge()`.
  Do not add another path to `DatabaseFleet::delete`.
- Notes are append-only. The backup check (`check_backup`) relies on it.
- Do not hold a `FleetDatabase` across requests or in a cache: every live
  handle pins an open file and defeats `max_open`.
- Do not `use diesel::prelude::*` in a module that touches the model macros or
  `diesel_async::RunQueryDsl`; import the derives you need (E0034 otherwise).
- `autumn.toml` refuses unknown keys: Apiary's own settings are `APIARY_*`
  environment variables (`src/settings.rs`).
- Every route must be registered in `main.rs` **and** listed in
  `routes::ALL` (a test checks).

## Gates

`./verify.sh` (authoritative, on the Mac). In the cloud:
`cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test --workspace`.
