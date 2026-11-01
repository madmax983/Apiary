# ADR 0002 — One migration set, applied to control and to every hive

Status: accepted (a workaround), 2026-10-31

## Context

Apiary has tenant data (`notes`, belongs in each hive) and control-plane data
(`offboardings`, belongs in the control database only). autumn-web 0.9 applies
every app migration set registered with `AppBuilder::migrations` to the control
database **and** to every fleet database when it opens; only the framework's
own control-plane sets are kept off the fleet. There is no way to mark an app
set control-only or fleet-only.

## Decision

Register one set with both tables. Each hive file therefore carries an empty
`offboardings` table, and the control database an empty `notes` table.

Guarded by `control_and_hive_tables_stay_apart` in `tests/app.rs`: control
holds no notes, and a hive holds no offboarding rows.

## Alternatives considered

- Create `offboardings` by hand on the control pool in the startup hook. Works,
  but takes the table out of the migration history.
- Store offboarding status only in the Harvest runtime's own file. Ties the
  hive page to the workflow engine's query API for a single row.

## Revisit when

autumn-web grows a way to scope an app migration set (for example
`.migrations_for(Control, …)` / `.migrations_for(Fleet, …)`). Then split the
set and drop the empty copies.

That way now exists, after autumn-web 0.9.0: `AppBuilder::migrations_for`
with `MigrationScope::Control` and `MigrationScope::Shards` ([autumn#3559](https://github.com/autumn-foundation/autumn/pull/3559)). Do
the split when Apiary upgrades.
