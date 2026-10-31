// Apiary — the verified core, part one: nothing is deleted that cannot be restored.
//
//   verus verus/src/offboard.rs
//
// The one irreversible thing Apiary does is delete a hive's SQLite file. It
// only ever does it at the end of an offboarding, and the offboarding is the
// state machine in `crates/hive/src/offboard.rs`. This file is that machine,
// transition for transition, and the property that makes it worth having:
//
//   In every state reachable from `init` by any sequence of steps, if the
//   phase is `Purging` or `Deleted`, then a backup was verified, the grace
//   period elapsed, and nobody asked to keep the hive.
//
// plus: `Kept`, `Aborted` and `Deleted` are terminal (no step leaves them),
// and the executable `apply` below computes exactly the spec `step`.
//
// ── The modelling assumption ────────────────────────────────────────────────
//
// The runtime type keeps `phase` private and folds steps through `apply`, so
// a value of it is always the result of `replay` from `new()`; here that is
// `run(init(), steps)`. The runtime `apply` is `const fn` over a tuple match;
// this one is a nested match with the same arms. SPEC.md section 3 lists the
// arms side by side. The property test `every_reachable_state_is_safe` runs
// the runtime type against the same invariant (`is_safe` there, `inv` here).

use vstd::prelude::*;

verus! {

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Active,
    Snapshotting,
    Verifying,
    Grace,
    Kept,
    Aborted,
    Purging,
    Deleted,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Begin,
    SnapshotTaken,
    SnapshotVerified { ok: bool },
    Keep,
    GraceElapsed,
    Purged,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Offboarding {
    pub phase: Phase,
    pub verified: bool,
    pub grace_elapsed: bool,
    pub kept: bool,
}

pub open spec fn init() -> Offboarding {
    Offboarding { phase: Phase::Active, verified: false, grace_elapsed: false, kept: false }
}

pub open spec fn with_phase(s: Offboarding, p: Phase) -> Offboarding {
    Offboarding { phase: p, verified: s.verified, grace_elapsed: s.grace_elapsed, kept: s.kept }
}

/// SPEC 3.1. The transition table. `None` is a refused step.
pub open spec fn step(s: Offboarding, e: Step) -> Option<Offboarding> {
    match e {
        Step::Begin => if s.phase == Phase::Active {
            Some(with_phase(s, Phase::Snapshotting))
        } else {
            None
        },
        Step::SnapshotTaken => if s.phase == Phase::Snapshotting {
            Some(with_phase(s, Phase::Verifying))
        } else {
            None
        },
        Step::SnapshotVerified { ok } => if s.phase == Phase::Verifying {
            if ok {
                Some(Offboarding {
                    phase: Phase::Grace,
                    verified: true,
                    grace_elapsed: s.grace_elapsed,
                    kept: s.kept,
                })
            } else {
                Some(with_phase(s, Phase::Aborted))
            }
        } else {
            None
        },
        Step::Keep => if s.phase == Phase::Snapshotting || s.phase == Phase::Verifying
            || s.phase == Phase::Grace {
            Some(Offboarding {
                phase: Phase::Kept,
                verified: s.verified,
                grace_elapsed: s.grace_elapsed,
                kept: true,
            })
        } else {
            None
        },
        Step::GraceElapsed => if s.phase == Phase::Grace {
            Some(Offboarding {
                phase: Phase::Purging,
                verified: s.verified,
                grace_elapsed: true,
                kept: s.kept,
            })
        } else {
            None
        },
        Step::Purged => if s.phase == Phase::Purging {
            Some(with_phase(s, Phase::Deleted))
        } else {
            None
        },
    }
}

/// Fold a sequence of steps; `None` as soon as one is refused.
pub open spec fn run(s: Offboarding, steps: Seq<Step>) -> Option<Offboarding>
    decreases steps.len(),
{
    if steps.len() == 0 {
        Some(s)
    } else {
        match step(s, steps[0]) {
            Some(t) => run(t, steps.skip(1)),
            None => None,
        }
    }
}

/// SPEC 3.2. The inductive invariant. The first arm is the theorem; the other
/// arms are what it takes to carry the first one through every step.
pub open spec fn inv(s: Offboarding) -> bool {
    if s.phase == Phase::Purging || s.phase == Phase::Deleted {
        s.verified && s.grace_elapsed && !s.kept
    } else if s.phase == Phase::Grace {
        s.verified && !s.grace_elapsed && !s.kept
    } else if s.phase == Phase::Kept {
        s.kept
    } else {
        !s.grace_elapsed && !s.kept
    }
}

pub open spec fn terminal(p: Phase) -> bool {
    p == Phase::Kept || p == Phase::Aborted || p == Phase::Deleted
}

pub proof fn lemma_init_inv()
    ensures
        inv(init()),
{
}

pub proof fn lemma_step_preserves_inv(s: Offboarding, e: Step, t: Offboarding)
    requires
        inv(s),
        step(s, e) == Some(t),
    ensures
        inv(t),
{
    match e {
        Step::Begin => {},
        Step::SnapshotTaken => {},
        Step::SnapshotVerified { ok } => {},
        Step::Keep => {},
        Step::GraceElapsed => {},
        Step::Purged => {},
    }
}

pub proof fn lemma_run_preserves_inv(s: Offboarding, steps: Seq<Step>, t: Offboarding)
    requires
        inv(s),
        run(s, steps) == Some(t),
    ensures
        inv(t),
    decreases steps.len(),
{
    if steps.len() > 0 {
        let mid = step(s, steps[0]);
        assert(mid is Some);
        let m = mid->Some_0;
        lemma_step_preserves_inv(s, steps[0], m);
        lemma_run_preserves_inv(m, steps.skip(1), t);
    }
}

/// SPEC 3.3. **The theorem.** Whatever happened, if the file may be deleted
/// or has been, a verified backup exists, grace elapsed and nobody said keep.
pub proof fn theorem_no_unsafe_delete(steps: Seq<Step>, t: Offboarding)
    requires
        run(init(), steps) == Some(t),
        t.phase == Phase::Purging || t.phase == Phase::Deleted,
    ensures
        t.verified,
        t.grace_elapsed,
        !t.kept,
{
    lemma_init_inv();
    lemma_run_preserves_inv(init(), steps, t);
}

/// SPEC 3.4. `Kept`, `Aborted` and `Deleted` are final.
pub proof fn lemma_terminal_is_final(s: Offboarding, e: Step)
    requires
        terminal(s.phase),
    ensures
        step(s, e) is None,
{
    match e {
        Step::Begin => {},
        Step::SnapshotTaken => {},
        Step::SnapshotVerified { ok } => {},
        Step::Keep => {},
        Step::GraceElapsed => {},
        Step::Purged => {},
    }
}

/// SPEC 3.5. A keep is never too early to win: from any non-terminal phase
/// before the purge decision, `Keep` moves to `Kept`.
pub proof fn lemma_keep_wins_before_purging(s: Offboarding)
    requires
        s.phase == Phase::Snapshotting || s.phase == Phase::Verifying || s.phase
            == Phase::Grace,
    ensures
        step(s, Step::Keep) is Some,
        (step(s, Step::Keep)->Some_0).phase == Phase::Kept,
{
}

/// The gate the purge activity sits behind.
pub open spec fn may_purge(s: Offboarding) -> bool {
    s.phase == Phase::Purging
}

/// SPEC 3.6. The executable step, as `crates/hive/src/offboard.rs` writes it.
pub fn apply(s: Offboarding, e: Step) -> (r: Option<Offboarding>)
    ensures
        r == step(s, e),
{
    match e {
        Step::Begin => match s.phase {
            Phase::Active => Some(
                Offboarding {
                    phase: Phase::Snapshotting,
                    verified: s.verified,
                    grace_elapsed: s.grace_elapsed,
                    kept: s.kept,
                },
            ),
            _ => None,
        },
        Step::SnapshotTaken => match s.phase {
            Phase::Snapshotting => Some(
                Offboarding {
                    phase: Phase::Verifying,
                    verified: s.verified,
                    grace_elapsed: s.grace_elapsed,
                    kept: s.kept,
                },
            ),
            _ => None,
        },
        Step::SnapshotVerified { ok } => match s.phase {
            Phase::Verifying => if ok {
                Some(
                    Offboarding {
                        phase: Phase::Grace,
                        verified: true,
                        grace_elapsed: s.grace_elapsed,
                        kept: s.kept,
                    },
                )
            } else {
                Some(
                    Offboarding {
                        phase: Phase::Aborted,
                        verified: s.verified,
                        grace_elapsed: s.grace_elapsed,
                        kept: s.kept,
                    },
                )
            },
            _ => None,
        },
        Step::Keep => match s.phase {
            Phase::Snapshotting | Phase::Verifying | Phase::Grace => Some(
                Offboarding {
                    phase: Phase::Kept,
                    verified: s.verified,
                    grace_elapsed: s.grace_elapsed,
                    kept: true,
                },
            ),
            _ => None,
        },
        Step::GraceElapsed => match s.phase {
            Phase::Grace => Some(
                Offboarding {
                    phase: Phase::Purging,
                    verified: s.verified,
                    grace_elapsed: true,
                    kept: s.kept,
                },
            ),
            _ => None,
        },
        Step::Purged => match s.phase {
            Phase::Purging => Some(
                Offboarding {
                    phase: Phase::Deleted,
                    verified: s.verified,
                    grace_elapsed: s.grace_elapsed,
                    kept: s.kept,
                },
            ),
            _ => None,
        },
    }
}

/// SPEC 3.7. The executable gate: `true` only when the theorem's conclusion
/// holds for any state reached by `apply` from `init`.
pub fn may_purge_exec(s: Offboarding) -> (r: bool)
    ensures
        r == may_purge(s),
{
    match s.phase {
        Phase::Purging => true,
        _ => false,
    }
}

fn main() {
}

} // verus!
