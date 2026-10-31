//! Offboarding: the only path by which a hive's database file is deleted.
//!
//! ```text
//!  Active ──Begin──► Snapshotting ──SnapshotTaken──► Verifying
//!                        │                              │  SnapshotVerified{ok: true}
//!                       Keep                            ├──────────────────────► Grace
//!                        ▼                              │  SnapshotVerified{ok: false}
//!                      Kept ◄────────Keep───────────────┤──────────────────────► Aborted
//!                        ▲
//!                        └───────────Keep──── Grace ──GraceElapsed──► Purging ──Purged──► Deleted
//! ```
//!
//! The workflow in the app crate feeds every outcome through [`Offboarding::apply`]
//! and calls the delete activity only when [`Offboarding::may_purge`] says so.
//! The safety property, proved in `verus/src/offboard.rs` over the same
//! transition table:
//!
//! > In every reachable state whose phase is `Purging` or `Deleted`, a backup
//! > was verified, the grace period elapsed, and nobody asked to keep the hive.
//!
//! plus: `Kept`, `Aborted` and `Deleted` are terminal, and no single step from
//! `Active` reaches `Purging`.

use serde::{Deserialize, Serialize};

/// Where an offboarding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Nothing has happened; the hive is live.
    Active,
    /// A backup of the hive's file is being written to the archive.
    Snapshotting,
    /// The backup exists and is being checked against the live file.
    Verifying,
    /// The backup checked out. The hive is still live, waiting out the grace
    /// period in case somebody wants it kept.
    Grace,
    /// Somebody asked to keep the hive. Terminal.
    Kept,
    /// The backup did not check out, so nothing will be deleted. Terminal.
    Aborted,
    /// Grace elapsed with no keep: the file may be deleted now.
    Purging,
    /// The file is gone. Terminal.
    Deleted,
}

impl Phase {
    /// Whether no step leaves this phase.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Kept | Self::Aborted | Self::Deleted)
    }

    /// A short word for the interface.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Snapshotting => "snapshotting",
            Self::Verifying => "verifying",
            Self::Grace => "grace",
            Self::Kept => "kept",
            Self::Aborted => "aborted",
            Self::Purging => "purging",
            Self::Deleted => "deleted",
        }
    }
}

/// Something that happened to an offboarding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "step")]
pub enum Step {
    /// Somebody asked for the hive to be offboarded.
    Begin,
    /// The backup file was written.
    SnapshotTaken,
    /// The backup was checked: `ok` when it passed `PRAGMA integrity_check`
    /// and holds exactly the rows the live file holds.
    SnapshotVerified {
        /// Whether the backup passed.
        ok: bool,
    },
    /// Somebody asked to keep the hive.
    Keep,
    /// The grace period ran out with no keep.
    GraceElapsed,
    /// The file was deleted.
    Purged,
}

/// A step that is not allowed from where the offboarding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{step:?} is not allowed while {from:?}")]
pub struct StepError {
    /// Where the offboarding was.
    pub from: Phase,
    /// The step that was refused.
    pub step: Step,
}

/// An offboarding: its phase, plus the three facts the safety property is
/// about. The facts only ever turn on, never off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Offboarding {
    phase: Phase,
    verified: bool,
    grace_elapsed: bool,
    kept: bool,
}

impl Default for Offboarding {
    fn default() -> Self {
        Self::new()
    }
}

impl Offboarding {
    /// A hive nobody has asked to offboard.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: Phase::Active,
            verified: false,
            grace_elapsed: false,
            kept: false,
        }
    }

    /// Where it is.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether a backup has been verified.
    #[must_use]
    pub const fn verified(&self) -> bool {
        self.verified
    }

    /// Whether the grace period has elapsed.
    #[must_use]
    pub const fn grace_elapsed(&self) -> bool {
        self.grace_elapsed
    }

    /// Whether somebody asked to keep the hive.
    #[must_use]
    pub const fn kept(&self) -> bool {
        self.kept
    }

    /// Whether the hive's file may be deleted now. This is the gate the purge
    /// activity sits behind.
    #[must_use]
    pub const fn may_purge(&self) -> bool {
        matches!(self.phase, Phase::Purging)
    }

    /// The safety property, as a runtime check. Proved for every reachable
    /// state in `verus/src/offboard.rs`; asserted here by the property tests.
    #[must_use]
    pub const fn is_safe(&self) -> bool {
        match self.phase {
            Phase::Purging | Phase::Deleted => self.verified && self.grace_elapsed && !self.kept,
            Phase::Grace => self.verified && !self.grace_elapsed && !self.kept,
            Phase::Kept => self.kept,
            _ => !self.grace_elapsed && !self.kept,
        }
    }

    /// Take one step.
    ///
    /// # Errors
    ///
    /// [`StepError`] when `step` is not allowed from the current phase. The
    /// offboarding is unchanged then.
    pub const fn apply(self, step: Step) -> Result<Self, StepError> {
        let next = match (self.phase, step) {
            (Phase::Active, Step::Begin) => Self {
                phase: Phase::Snapshotting,
                ..self
            },
            (Phase::Snapshotting, Step::SnapshotTaken) => Self {
                phase: Phase::Verifying,
                ..self
            },
            (Phase::Verifying, Step::SnapshotVerified { ok: true }) => Self {
                phase: Phase::Grace,
                verified: true,
                ..self
            },
            (Phase::Verifying, Step::SnapshotVerified { ok: false }) => Self {
                phase: Phase::Aborted,
                ..self
            },
            (Phase::Snapshotting | Phase::Verifying | Phase::Grace, Step::Keep) => Self {
                phase: Phase::Kept,
                kept: true,
                ..self
            },
            (Phase::Grace, Step::GraceElapsed) => Self {
                phase: Phase::Purging,
                grace_elapsed: true,
                ..self
            },
            (Phase::Purging, Step::Purged) => Self {
                phase: Phase::Deleted,
                ..self
            },
            (from, step) => return Err(StepError { from, step }),
        };
        Ok(next)
    }

    /// Fold a sequence of steps, stopping at the first refused one.
    ///
    /// # Errors
    ///
    /// The first [`StepError`].
    pub fn replay(steps: impl IntoIterator<Item = Step>) -> Result<Self, StepError> {
        steps.into_iter().try_fold(Self::new(), Self::apply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const HAPPY: [Step; 5] = [
        Step::Begin,
        Step::SnapshotTaken,
        Step::SnapshotVerified { ok: true },
        Step::GraceElapsed,
        Step::Purged,
    ];

    #[test]
    fn the_happy_path_deletes() {
        let o = Offboarding::replay(HAPPY).unwrap();
        assert_eq!(o.phase(), Phase::Deleted);
        assert!(o.verified() && o.grace_elapsed() && !o.kept());
    }

    #[test]
    fn purge_is_allowed_only_in_purging() {
        let mut o = Offboarding::new();
        for step in &HAPPY[..4] {
            assert!(!o.may_purge(), "{:?}", o.phase());
            o = o.apply(*step).unwrap();
        }
        assert!(o.may_purge());
        assert!(!o.apply(Step::Purged).unwrap().may_purge());
    }

    #[test]
    fn a_failed_check_aborts_and_never_deletes() {
        let o = Offboarding::replay([
            Step::Begin,
            Step::SnapshotTaken,
            Step::SnapshotVerified { ok: false },
        ])
        .unwrap();
        assert_eq!(o.phase(), Phase::Aborted);
        assert!(o.apply(Step::GraceElapsed).is_err());
        assert!(o.apply(Step::Purged).is_err());
    }

    #[test]
    fn keep_wins_during_grace_and_is_final() {
        let o = Offboarding::replay([
            Step::Begin,
            Step::SnapshotTaken,
            Step::SnapshotVerified { ok: true },
            Step::Keep,
        ])
        .unwrap();
        assert_eq!(o.phase(), Phase::Kept);
        for step in HAPPY {
            assert!(o.apply(step).is_err(), "{step:?} after Keep");
        }
    }

    #[test]
    fn keep_is_too_late_once_purging() {
        let o = Offboarding::replay(HAPPY[..4].iter().copied()).unwrap();
        assert_eq!(
            o.apply(Step::Keep),
            Err(StepError {
                from: Phase::Purging,
                step: Step::Keep
            })
        );
    }

    #[test]
    fn grace_cannot_be_skipped() {
        let o = Offboarding::replay([
            Step::Begin,
            Step::SnapshotTaken,
            Step::SnapshotVerified { ok: true },
        ])
        .unwrap();
        assert!(o.apply(Step::Purged).is_err());
    }

    #[test]
    fn a_refused_step_names_where_it_was() {
        assert_eq!(
            Offboarding::new().apply(Step::Purged),
            Err(StepError {
                from: Phase::Active,
                step: Step::Purged
            })
        );
    }

    #[test]
    fn terminal_phases_are_exactly_the_three() {
        let terminal: Vec<Phase> = [
            Phase::Active,
            Phase::Snapshotting,
            Phase::Verifying,
            Phase::Grace,
            Phase::Kept,
            Phase::Aborted,
            Phase::Purging,
            Phase::Deleted,
        ]
        .into_iter()
        .filter(|p| p.is_terminal())
        .collect();
        assert_eq!(terminal, [Phase::Kept, Phase::Aborted, Phase::Deleted]);
    }

    fn any_step() -> impl Strategy<Value = Step> {
        prop_oneof![
            Just(Step::Begin),
            Just(Step::SnapshotTaken),
            Just(Step::SnapshotVerified { ok: true }),
            Just(Step::SnapshotVerified { ok: false }),
            Just(Step::Keep),
            Just(Step::GraceElapsed),
            Just(Step::Purged),
        ]
    }

    proptest! {
        /// The Verus theorem, run: whatever is thrown at it, every state the
        /// machine reaches is safe, and a refused step changes nothing.
        #[test]
        fn every_reachable_state_is_safe(steps in proptest::collection::vec(any_step(), 0..40)) {
            let mut o = Offboarding::new();
            prop_assert!(o.is_safe());
            for step in steps {
                match o.apply(step) {
                    Ok(next) => {
                        prop_assert!(next.is_safe(), "{:?} --{:?}--> {:?}", o, step, next);
                        prop_assert!(!o.phase().is_terminal(), "left terminal {:?}", o.phase());
                        o = next;
                    }
                    Err(e) => prop_assert_eq!(e.from, o.phase()),
                }
            }
        }

        /// Deleted is reachable only through the whole happy path, in order.
        #[test]
        fn deleted_implies_the_happy_path(steps in proptest::collection::vec(any_step(), 0..40)) {
            let mut o = Offboarding::new();
            let mut accepted = Vec::new();
            for step in steps {
                if let Ok(next) = o.apply(step) {
                    accepted.push(step);
                    o = next;
                }
            }
            if o.phase() == Phase::Deleted {
                prop_assert_eq!(accepted, HAPPY.to_vec());
            }
        }

        /// The facts never turn off.
        #[test]
        fn facts_are_monotone(steps in proptest::collection::vec(any_step(), 0..40)) {
            let mut o = Offboarding::new();
            for step in steps {
                if let Ok(next) = o.apply(step) {
                    prop_assert!(!o.verified() || next.verified());
                    prop_assert!(!o.grace_elapsed() || next.grace_elapsed());
                    prop_assert!(!o.kept() || next.kept());
                    o = next;
                }
            }
        }
    }
}
