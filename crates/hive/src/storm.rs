//! The storm: a deterministic workload that churns far more hives than the
//! fleet may keep open, so the least-recently-used ones are closed and
//! re-opened in front of you.
//!
//! A plan is two halves:
//!
//! 1. [`StormPlan::picks`] — which hive each touch lands on, and whether it
//!    writes or reads. A seeded xorshift with a hot set: most touches land on a
//!    few hives, the rest anywhere, which is the shape that makes an LRU work
//!    hard.
//! 2. [`expand`] — turns picks into [`Op`]s, inserting a `Provision` the first
//!    time a hive is touched. The fleet runs with `create_on_demand = false`,
//!    so touching an unprovisioned hive is a `404`; `expand` makes that
//!    impossible, and `verus/src/storm.rs` proves it.

use std::collections::BTreeSet;

/// The most hives one storm may span.
pub const MAX_HIVES: u32 = 9_999;
/// The most touches one storm may make.
pub const MAX_TOUCHES: u32 = 200_000;

/// One thing the storm does to the fleet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    /// Create hive `n`'s database.
    Provision(u32),
    /// Write note `seq` (the hive's own counter, from 1) into hive `n`.
    Write {
        /// Which hive.
        hive: u32,
        /// The hive's own write counter, from 1.
        seq: u32,
    },
    /// Read hive `n`'s notes.
    Read(u32),
}

impl Op {
    /// The hive the op is on.
    #[must_use]
    pub const fn hive(&self) -> u32 {
        match *self {
            Self::Provision(h) | Self::Read(h) | Self::Write { hive: h, .. } => h,
        }
    }
}

/// Why a plan was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StormPlanError {
    /// No hives, or more than [`MAX_HIVES`].
    #[error("a storm spans 1 to {MAX_HIVES} hives")]
    Hives,
    /// No touches, or more than [`MAX_TOUCHES`].
    #[error("a storm makes 1 to {MAX_TOUCHES} touches")]
    Touches,
    /// A hot set bigger than the population.
    #[error("the hot set cannot be bigger than the population")]
    Hot,
    /// A percentage over 100.
    #[error("percentages are 0 to 100")]
    Percent,
}

/// The shape of a storm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StormPlan {
    hives: u32,
    touches: u32,
    hot: u32,
    hot_percent: u32,
    write_percent: u32,
    seed: u64,
}

impl StormPlan {
    /// A plan over `hives` hives making `touches` touches; `hot` of the hives
    /// take `hot_percent` of the touches, and `write_percent` of touches
    /// write. The same arguments always give the same ops.
    ///
    /// # Errors
    ///
    /// A [`StormPlanError`] naming the bad argument.
    pub const fn new(
        hives: u32,
        touches: u32,
        hot: u32,
        hot_percent: u32,
        write_percent: u32,
        seed: u64,
    ) -> Result<Self, StormPlanError> {
        if hives == 0 || hives > MAX_HIVES {
            return Err(StormPlanError::Hives);
        }
        if touches == 0 || touches > MAX_TOUCHES {
            return Err(StormPlanError::Touches);
        }
        if hot > hives {
            return Err(StormPlanError::Hot);
        }
        if hot_percent > 100 || write_percent > 100 {
            return Err(StormPlanError::Percent);
        }
        Ok(Self {
            hives,
            touches,
            hot,
            hot_percent,
            write_percent,
            seed,
        })
    }

    /// The storm the button runs: 1,000 hives, 6,000 touches, 24 hot hives
    /// taking 70% of them — against a fleet that keeps 16 open.
    #[must_use]
    pub const fn default_storm(seed: u64) -> Self {
        Self {
            hives: 1_000,
            touches: 6_000,
            hot: 24,
            hot_percent: 70,
            write_percent: 60,
            seed,
        }
    }

    /// How many hives the storm spans.
    #[must_use]
    pub const fn hives(&self) -> u32 {
        self.hives
    }

    /// How many touches it makes.
    #[must_use]
    pub const fn touches(&self) -> u32 {
        self.touches
    }

    /// Which hive each touch lands on (`0..hives`) and whether it writes.
    #[must_use]
    pub fn picks(&self) -> Vec<(u32, bool)> {
        let mut rng = XorShift::new(self.seed);
        (0..self.touches)
            .map(|_| {
                let hive = if self.hot > 0 && rng.below(100) < self.hot_percent {
                    rng.below(self.hot)
                } else {
                    rng.below(self.hives)
                };
                let write = rng.below(100) < self.write_percent;
                (hive, write)
            })
            .collect()
    }

    /// The whole storm.
    #[must_use]
    pub fn ops(&self) -> Vec<Op> {
        expand(&self.picks(), self.hives)
    }
}

/// Turn picks into ops: provision a hive the first time it is touched, and
/// number each hive's writes from 1. Picks naming a hive at or past `hives`
/// are dropped.
///
/// Proved in `verus/src/storm.rs`: every `Write` or `Read` on hive `h` is
/// preceded by exactly one `Provision(h)`, and no hive is provisioned twice.
#[must_use]
pub fn expand(picks: &[(u32, bool)], hives: u32) -> Vec<Op> {
    let mut provisioned = vec![false; hives as usize];
    let mut writes = vec![0u32; hives as usize];
    let mut out = Vec::with_capacity(picks.len() + picks.len() / 4);
    for &(hive, write) in picks {
        let i = hive as usize;
        if i >= provisioned.len() {
            continue;
        }
        if !provisioned[i] {
            provisioned[i] = true;
            out.push(Op::Provision(hive));
        }
        if write {
            writes[i] = writes[i].saturating_add(1);
            out.push(Op::Write {
                hive,
                seq: writes[i],
            });
        } else {
            out.push(Op::Read(hive));
        }
    }
    out
}

/// Check the property `expand` is proved to have, on any op list. Used by the
/// property tests, and by the storm runner as a debug assertion.
#[must_use]
pub fn provisions_first(ops: &[Op]) -> bool {
    let mut seen = BTreeSet::new();
    ops.iter().all(|op| match *op {
        Op::Provision(h) => seen.insert(h),
        Op::Write { hive, .. } | Op::Read(hive) => seen.contains(&hive),
    })
}

/// xorshift64* — small, fast, and the same on every platform.
#[derive(Debug, Clone)]
struct XorShift(u64);

impl XorShift {
    const fn new(seed: u64) -> Self {
        // Zero is the one state xorshift never leaves.
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    const fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-enough in `0..n` for a workload; `n > 0`.
    fn below(&mut self, n: u32) -> u32 {
        // Lemire's multiply-shift: no modulo bias worth speaking of at u32.
        let wide = u64::from((self.next() >> 32) as u32) * u64::from(n);
        u32::try_from(wide >> 32).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_default_storm_touches_more_hives_than_any_fleet_keeps_open() {
        let plan = StormPlan::default_storm(7);
        let ops = plan.ops();
        let distinct: BTreeSet<u32> = ops.iter().map(Op::hive).collect();
        assert!(
            distinct.len() > 400,
            "only {} hives touched; the LRU would barely work",
            distinct.len()
        );
        assert!(provisions_first(&ops));
        let touches = ops
            .iter()
            .filter(|o| !matches!(o, Op::Provision(_)))
            .count();
        assert_eq!(touches, 6_000);
    }

    #[test]
    fn the_hot_set_is_hot() {
        let picks = StormPlan::default_storm(1).picks();
        let hot = picks.iter().filter(|(h, _)| *h < 24).count();
        // 70% aimed at the hot set, plus the ~2.4% of cold picks that land there.
        assert!((4_000..4_600).contains(&hot), "{hot}");
    }

    #[test]
    fn seeds_decide_everything() {
        assert_eq!(
            StormPlan::default_storm(3).ops(),
            StormPlan::default_storm(3).ops()
        );
        assert_ne!(
            StormPlan::default_storm(3).ops(),
            StormPlan::default_storm(4).ops()
        );
    }

    #[test]
    fn a_zero_seed_still_storms() {
        let ops = StormPlan::default_storm(0).ops();
        assert!(ops.iter().map(Op::hive).collect::<BTreeSet<_>>().len() > 100);
    }

    #[test]
    fn expand_numbers_writes_per_hive() {
        let ops = expand(&[(2, true), (5, true), (2, false), (2, true)], 10);
        assert_eq!(
            ops,
            vec![
                Op::Provision(2),
                Op::Write { hive: 2, seq: 1 },
                Op::Provision(5),
                Op::Write { hive: 5, seq: 1 },
                Op::Read(2),
                Op::Write { hive: 2, seq: 2 },
            ]
        );
    }

    #[test]
    fn expand_drops_picks_outside_the_population() {
        assert_eq!(
            expand(&[(10, true), (0, false)], 10),
            vec![Op::Provision(0), Op::Read(0)]
        );
    }

    #[test]
    fn plans_refuse_bad_shapes() {
        assert_eq!(StormPlan::new(0, 1, 0, 0, 0, 1), Err(StormPlanError::Hives));
        assert_eq!(
            StormPlan::new(10_000, 1, 0, 0, 0, 1),
            Err(StormPlanError::Hives)
        );
        assert_eq!(
            StormPlan::new(1, 0, 0, 0, 0, 1),
            Err(StormPlanError::Touches)
        );
        assert_eq!(StormPlan::new(5, 1, 6, 0, 0, 1), Err(StormPlanError::Hot));
        assert_eq!(
            StormPlan::new(5, 1, 5, 101, 0, 1),
            Err(StormPlanError::Percent)
        );
        assert!(StormPlan::new(5, 1, 5, 100, 100, 1).is_ok());
    }

    proptest! {
        /// The Verus theorem, run on arbitrary picks.
        #[test]
        fn expand_always_provisions_first(
            picks in proptest::collection::vec((0u32..50, any::<bool>()), 0..500),
            hives in 1u32..60,
        ) {
            let ops = expand(&picks, hives);
            prop_assert!(provisions_first(&ops));
            prop_assert!(ops.iter().all(|o| o.hive() < hives));
            let kept = picks.iter().filter(|(h, _)| *h < hives).count();
            let touches = ops.iter().filter(|o| !matches!(o, Op::Provision(_))).count();
            prop_assert_eq!(touches, kept);
        }

        /// Every pick lands in the population, for any plan.
        #[test]
        fn picks_stay_in_range(
            hives in 1u32..200, hot_frac in 0u32..=100, hp in 0u32..=100,
            wp in 0u32..=100, seed in any::<u64>(),
        ) {
            let hot = hives * hot_frac / 100;
            let plan = StormPlan::new(hives, 300, hot, hp, wp, seed).unwrap();
            prop_assert!(plan.picks().iter().all(|(h, _)| *h < hives));
        }

        /// Writes are numbered 1, 2, 3, … per hive, with no gaps.
        #[test]
        fn write_seqs_count_up(seed in any::<u64>()) {
            let plan = StormPlan::new(30, 400, 5, 60, 70, seed).unwrap();
            let mut last = std::collections::BTreeMap::new();
            for op in plan.ops() {
                if let Op::Write { hive, seq } = op {
                    let prev = last.insert(hive, seq).unwrap_or(0);
                    prop_assert_eq!(seq, prev + 1);
                }
            }
        }
    }
}
