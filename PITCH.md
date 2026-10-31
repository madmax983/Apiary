# Apiary

**One SQLite hive per tenant, and a window onto the fleet.**

autumn-web 0.9 shipped a database fleet: give every tenant its own SQLite
file, and the framework opens it on first use, migrates it, and closes it when
it sits idle or when too many are open. The single process-wide SQLite writer
becomes one writer per tenant, and every tenant becomes a file you can copy,
restore or delete on its own.

That is invisible by design: a handler declares
`#[repository(Note, tenant_scoped, sharded)]` and never learns any of it. So
Apiary makes it visible.

## The moment

You open the comb — one hexagonal cell per tenant file — and press **Raise a
storm**. A thousand hives are founded in front of you. The cells flood with
honey-light as their files open, ember as they close, and the gauge beside them
reads the fleet's own count: *261 / 16*. The cap you configured is sixteen.
Then the storm moves on and, a few seconds later, the gauge settles back to
sixteen and then to zero. You have just watched `max_open` behave as what it
is: a soft ceiling under bursts, held by a two-second residency grace, and
restored by a sweeper.

Then you walk into one hive, offboard it, and watch a durable workflow back its
file up, check the backup on its own, and count down a grace period. You can
keep it. If you do not, the file is deleted — and its ghost cell on the comb
still lets you walk back in and restore it.

## What is in it

- **The comb** — a live map of every hive file, pushed over SSE at most four
  times a second however hard the fleet is working.
- **The storm** — a seeded workload over 1,000 hives with a hot set of 24,
  provisioned-before-touched by construction (proved in Verus).
- **Search every hive** — the same generated repository run over every file
  with `CrossShard`; the comb lights up in a wave as it sweeps.
- **Offboarding** — a durable workflow on `autumn-harvest-sqlite`. The one
  irreversible thing Apiary does sits behind a state machine whose safety is
  proved in Verus: nothing is deleted without a verified backup, an elapsed
  grace, and no keep.
- **Restore** — the backup goes back through the fleet's own lifecycle hook,
  under its path lock, and is identity-checked and migrated like any file.

## Where it could go

- Slot mode: the same app with `mode = "slot"` — many tenants per file — and a
  comb of 8,192 slots.
- Replication and ownership: two hosts splitting the slot range, with the comb
  showing which host owns which cells and a fresh volume restoring on first
  touch.
- A replay of the storm at a slower pace, so the LRU closing can be watched one
  cell at a time.
