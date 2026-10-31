# Apiary — specification

What must be **formally** true (proved in `verus/`), what must be
**observably** true (tested), and an honest scoreboard of which proofs have
actually been run.

## 1. The fleet as Apiary uses it

Configured in `autumn.toml`: `[database.fleet]` with `mode = "tenant"`,
`max_open = 16`, `pool_size = 2`, `create_on_demand = false`,
`idle_close_secs = 20`. The top-level `database.url` is the control database.

Observable (tests in `crates/apiary-app/tests/app.rs`):

| # | Property | Test |
|---|----------|------|
| 1.1 | Founding a hive creates exactly one file, `tenant:<name>`, under the fleet root; founding it again creates nothing. | `founding_a_hive_creates_exactly_its_file` |
| 1.2 | A note written in hive A is in A's file and in no other hive's, read back through the generated sharded repository. | `notes_go_through_the_sharded_repository_into_each_hives_own_file` |
| 1.3 | An unfounded hive is `404`; a name that cannot name a file is `400`; neither creates a file. | `an_unfounded_hive_is_a_404_and_a_hostile_one_a_400` |
| 1.4 | `CrossShard` runs the repository over every file. | `search_runs_the_repository_across_every_hive` |
| 1.5 | Export is a real SQLite file that passes the backup check. | `export_is_a_real_sqlite_file_holding_the_hives_notes` |
| 1.6 | Control holds no notes; hives hold no offboarding rows. | `control_and_hive_tables_stay_apart` |
| 1.7 | `max_open` is **soft** under bursts: a file used in the last `min_residency` (2 s in a config-built app) is never closed to make room. After the burst the sweeper brings the open count back to `max_open`, then to 0 after `idle_close_secs`. | observed live (§6); the tests build fleets with `min_residency = 0` |

## 2. Hive names (`crates/hive/src/name.rs`)

`[a-z][a-z0-9-]*`, ≤ 40 bytes, no `--`, no trailing `-`, not a Windows device
name.

- **2.1** Every accepted name is a fleet key naming a file under the root.
  *Property test against the real `DatabaseFleet`* (`tests/name_vs_fleet.rs`,
  2,000 cases).
- **2.2** Whatever the fleet refuses, `HiveName` refuses first. Same file.
- **2.3** No accepted name contains `/`, `\`, `.` or whitespace. Property test.

Not proved: the fleet's key rules are the framework's, and are what this edge
checks against rather than models.

## 3. Offboarding (`crates/hive/src/offboard.rs`, `verus/src/offboard.rs`)

### 3.1 Transition table

| From | Step | To | Facts set |
|------|------|----|-----------|
| Active | Begin | Snapshotting | |
| Snapshotting | SnapshotTaken | Verifying | |
| Verifying | SnapshotVerified{ok: true} | Grace | verified |
| Verifying | SnapshotVerified{ok: false} | Aborted | |
| Snapshotting, Verifying, Grace | Keep | Kept | kept |
| Grace | GraceElapsed | Purging | grace_elapsed |
| Purging | Purged | Deleted | |
| anything else | — | refused (`StepError`), state unchanged | |

The runtime `apply` (a `const fn` over a tuple match) and the Verus `apply`
(a nested match) list the same arms.

### 3.2 Invariant

```
Purging | Deleted  ⇒ verified ∧ grace_elapsed ∧ ¬kept
Grace              ⇒ verified ∧ ¬grace_elapsed ∧ ¬kept
Kept               ⇒ kept
otherwise          ⇒ ¬grace_elapsed ∧ ¬kept
```

### 3.3 The theorem — formal

For every sequence of steps `s`, if `run(init, s) = Some(t)` and `t` is
`Purging` or `Deleted`, then `t.verified ∧ t.grace_elapsed ∧ ¬t.kept`.
(`theorem_no_unsafe_delete`, by `lemma_init_inv` + induction in
`lemma_run_preserves_inv`.)

### 3.4 `Kept`, `Aborted`, `Deleted` are terminal — formal

### 3.5 Keep wins from Snapshotting, Verifying and Grace — formal

### 3.6 Executable `apply` computes the spec `step` — formal

### 3.7 `may_purge` ⇔ phase is `Purging` — formal

### 3.8 The workflow — observable

`offboard_hive` (`crates/apiary-app/src/offboard.rs`) feeds every outcome
through `Offboarding::apply` and schedules `purge_hive` only behind
`may_purge()`. The backup check: the backup opens read-only, `PRAGMA
integrity_check` returns exactly `ok`, and it holds at least as many notes as
the live file held just before the backup. Sound because notes are
append-only: there is no route that deletes one.

Tests (`tests/offboarding.rs`, one process, three scenarios): deleted only
after `grace`, with the recorded backup passing the check; restore puts every
note back; a keep during grace leaves the file in place past the end of the
grace; a second offboard while one runs starts no second workflow.

Runtime property tests mirror 3.3–3.4 on random step sequences:
`every_reachable_state_is_safe`, `deleted_implies_the_happy_path`,
`facts_are_monotone`.

## 4. The storm (`crates/hive/src/storm.rs`, `verus/src/storm.rs`)

- **4.1** `provisions_first`: every Write/Read on hive `h` comes after a
  Provision of `h`. **Formal**, for *any* picks.
- **4.2** `provisioned_once`: no hive is provisioned twice. **Formal**.
- **4.3** Every op names a hive in the population. **Formal**.
- **4.4** Same seed, same ops; writes numbered 1, 2, 3… per hive; the default
  storm touches > 400 distinct hives with ~70% of touches on the hot 24.
  Property/unit tests.
- **4.5** Against a real fleet keeping 3 open: every op lands, each hive holds
  exactly the notes the plan wrote to it and no other hive's. Test
  `a_small_storm_lands_every_op_in_the_right_file` (40 hives — small enough
  for a default macOS `ulimit -n`).

## 5. The map (`crates/hive/src/map.rs`)

Observable: the open count equals the set of hives whose last event was an
open, under any interleaving of known/open/close/delete (property test against
a set model); the peak never under-counts it.

## 6. Observed live (cloud, debug build)

The default storm (1,000 hives, 6,000 touches, 8 workers): 0 failed ops,
≈1,300 ops/s, 833 hives founded, peak open **594** against `max_open = 16`;
back to 16 within ~12 s and to 0 after `idle_close_secs`. A cross-hive search
read 834 files (3,613 notes) in ~2 s. Offboard → delete → restore round-trip
returned every note.

## 7. Scoreboard — none of the Verus has been run yet

Ranked by how likely each obligation is to need another pass on the Mac.

| Obligation | File | Confidence | Why |
|---|---|---|---|
| 3.3 theorem, 3.4 terminal, 3.5 keep | `offboard.rs` | high | Pure case splits over a finite table; the inductive step is one `match` on the step with empty arms. |
| 3.6 exec `apply` = spec `step` | `offboard.rs` | high | Arm-for-arm the same; struct literals spelled out (no `..s`). |
| `run` decreases / `->Some_0` | `offboard.rs` | medium-high | `->Some_0` is the field syntax of recent Verus; older builds spell it `get_Some_0()`. |
| 4.1–4.3 `expand` | `storm.rs` | medium | Quantifier triggers on `out@[j]` and the ghost `first` map; the `assert forall … by` blocks supply the witnesses, but trigger selection is the usual place a first run needs a nudge. `picks[p]` indexing a `Vec<(u32, bool)>` relies on Copy indexing support. |
| `filled` helper | `storm.rs` | medium-high | Standard loop invariant. |
