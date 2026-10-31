//! Running a storm against the live fleet.
//!
//! The plan comes from [`hive::storm`]; this module plays it. Ops are dealt to
//! [`WORKERS`] workers by hive, so each hive's ops keep their order (its
//! provision lands before its first touch) while different hives run at once.
//! After every op the runner reads the fleet's own counters and remembers the
//! most files it ever saw open, and whether that ever exceeded `max_open` —
//! the one number the storm exists to watch.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use autumn_web::db::fleet::{DatabaseFleet, FleetError};
use autumn_web::reexports::diesel;
use hive::{HiveName, Op, StormPlan};

/// How many hives are worked on at once.
pub const WORKERS: usize = 8;

/// Why one op failed.
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    /// The fleet refused.
    #[error(transparent)]
    Fleet(#[from] FleetError),
    /// The hive's pool had no connection.
    #[error("no connection: {0}")]
    Pool(String),
    /// The query failed.
    #[error(transparent)]
    Query(#[from] diesel::result::Error),
}

/// What the storm button shows.
#[derive(Debug, Default)]
pub struct StormState {
    running: AtomicBool,
    runs: AtomicU64,
    total: AtomicU64,
    done: AtomicU64,
    failed: AtomicU64,
    provisioned: AtomicU64,
    written: AtomicU64,
    read: AtomicU64,
    peak_open: AtomicUsize,
    over_cap: AtomicU64,
    max_open: AtomicUsize,
    elapsed_ms: AtomicU64,
    last_error: Mutex<Option<String>>,
}

/// A copy of [`StormState`] for drawing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StormReport {
    /// A storm is running now.
    pub running: bool,
    /// Storms started since boot.
    pub runs: u64,
    /// Ops in the current (or last) storm.
    pub total: u64,
    /// Ops finished.
    pub done: u64,
    /// Ops that failed.
    pub failed: u64,
    /// Hives created.
    pub provisioned: u64,
    /// Notes written.
    pub written: u64,
    /// Reads made.
    pub read: u64,
    /// The most hive files the fleet reported open at once.
    pub peak_open: usize,
    /// Samples where the fleet reported more files open than `max_open`.
    pub over_cap: u64,
    /// The fleet's `max_open`.
    pub max_open: usize,
    /// How long the storm took (or has taken so far).
    pub elapsed_ms: u64,
    /// The last failure, if any.
    pub last_error: Option<String>,
}

impl StormReport {
    /// Ops per second, over the storm so far.
    #[must_use]
    pub const fn rate(&self) -> u64 {
        match (self.done * 1_000).checked_div(self.elapsed_ms) {
            Some(r) => r,
            None => 0,
        }
    }
}

impl StormState {
    /// A copy for drawing.
    #[must_use]
    pub fn report(&self) -> StormReport {
        StormReport {
            running: self.running.load(Ordering::Acquire),
            runs: self.runs.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            done: self.done.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            provisioned: self.provisioned.load(Ordering::Relaxed),
            written: self.written.load(Ordering::Relaxed),
            read: self.read.load(Ordering::Relaxed),
            peak_open: self.peak_open.load(Ordering::Relaxed),
            over_cap: self.over_cap.load(Ordering::Relaxed),
            max_open: self.max_open.load(Ordering::Relaxed),
            elapsed_ms: self.elapsed_ms.load(Ordering::Relaxed),
            last_error: self
                .last_error
                .lock()
                .map_or_else(|p| p.into_inner().clone(), |g| g.clone()),
        }
    }

    /// Claim the storm. `false` when one is already running.
    fn claim(&self) -> bool {
        self.running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn reset(&self, total: u64, max_open: usize) {
        self.runs.fetch_add(1, Ordering::Relaxed);
        for counter in [
            &self.done,
            &self.failed,
            &self.provisioned,
            &self.written,
            &self.read,
            &self.over_cap,
            &self.elapsed_ms,
        ] {
            counter.store(0, Ordering::Relaxed);
        }
        self.total.store(total, Ordering::Relaxed);
        self.peak_open.store(0, Ordering::Relaxed);
        self.max_open.store(max_open, Ordering::Relaxed);
        self.set_error(None);
    }

    fn set_error(&self, e: Option<String>) {
        match self.last_error.lock() {
            Ok(mut g) => *g = e,
            Err(p) => *p.into_inner() = e,
        }
    }

    fn sample(&self, fleet: &DatabaseFleet) {
        let stats = fleet.stats();
        self.peak_open.fetch_max(stats.open, Ordering::Relaxed);
        if stats.open > stats.max_open {
            self.over_cap.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Start `plan` in the background. `false` when a storm is already running.
pub fn start(fleet: DatabaseFleet, state: Arc<StormState>, plan: StormPlan) -> bool {
    if !state.claim() {
        return false;
    }
    tokio::spawn(async move {
        run(&fleet, &state, plan).await;
        state.running.store(false, Ordering::Release);
    });
    true
}

/// Play `plan` to the end. Public so the integration test can run one in the
/// foreground.
pub async fn run(fleet: &DatabaseFleet, state: &Arc<StormState>, plan: StormPlan) {
    let ops = plan.ops();
    debug_assert!(hive::storm::provisions_first(&ops));
    state.reset(ops.len() as u64, fleet.max_open());
    let started = Instant::now();

    let mut lanes: Vec<Vec<Op>> = vec![Vec::new(); WORKERS];
    for op in ops {
        lanes[op.hive() as usize % WORKERS].push(op);
    }
    let mut set = tokio::task::JoinSet::new();
    for lane in lanes {
        let fleet = fleet.clone();
        let state = Arc::clone(state);
        set.spawn(async move {
            for op in lane {
                let result = play(&fleet, op).await;
                match result {
                    Ok(()) => match op {
                        Op::Provision(_) => state.provisioned.fetch_add(1, Ordering::Relaxed),
                        Op::Write { .. } => state.written.fetch_add(1, Ordering::Relaxed),
                        Op::Read(_) => state.read.fetch_add(1, Ordering::Relaxed),
                    },
                    Err(e) => {
                        state.set_error(Some(format!("{}: {e}", hive_name(op.hive()))));
                        state.failed.fetch_add(1, Ordering::Relaxed)
                    }
                };
                state.done.fetch_add(1, Ordering::Relaxed);
                state.sample(&fleet);
                state.elapsed_ms.store(
                    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    Ordering::Relaxed,
                );
            }
        });
    }
    while set.join_next().await.is_some() {}
    state.elapsed_ms.store(
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
}

/// Storm hive `n` (0-based in the plan) is `storm-0001` and on.
fn hive_name(n: u32) -> HiveName {
    HiveName::storm(n + 1)
}

async fn play(fleet: &DatabaseFleet, op: Op) -> Result<(), OpError> {
    use autumn_web::reexports::diesel_async::RunQueryDsl as _;

    let name = hive_name(op.hive());
    let key = fleet.key_for(name.as_str())?;
    match op {
        Op::Provision(_) => match fleet.provision(&key).await {
            // A second storm meets the first one's hives: that is fine.
            Ok(_) | Err(FleetError::AlreadyExists { .. }) => Ok(()),
            Err(e) => Err(e.into()),
        },
        Op::Write { seq, .. } => {
            let db = fleet.open(&key).await?;
            let _lease = db.lease();
            let mut conn = db
                .pool()
                .get()
                .await
                .map_err(|e| OpError::Pool(e.to_string()))?;
            diesel::sql_query("INSERT INTO notes (body, tenant_id, created_at) VALUES (?, ?, ?)")
                .bind::<diesel::sql_types::Text, _>(format!("storm note {seq}"))
                .bind::<diesel::sql_types::Text, _>(name.as_str())
                .bind::<diesel::sql_types::Timestamp, _>(chrono::Utc::now().naive_utc())
                .execute(&mut *conn)
                .await?;
            Ok(())
        }
        Op::Read(_) => {
            let db = fleet.open(&key).await?;
            let _lease = db.lease();
            let mut conn = db
                .pool()
                .get()
                .await
                .map_err(|e| OpError::Pool(e.to_string()))?;
            let _: crate::models::Count = diesel::sql_query("SELECT COUNT(*) AS n FROM notes")
                .get_result(&mut *conn)
                .await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_is_ops_per_second() {
        let r = StormReport {
            done: 3_000,
            elapsed_ms: 1_500,
            ..StormReport::default()
        };
        assert_eq!(r.rate(), 2_000);
        assert_eq!(StormReport::default().rate(), 0);
    }

    #[test]
    fn only_one_storm_at_a_time() {
        let s = StormState::default();
        assert!(s.claim());
        assert!(!s.claim());
        s.running.store(false, Ordering::Release);
        assert!(s.claim());
    }

    #[test]
    fn storm_hives_are_numbered_from_one() {
        assert_eq!(hive_name(0).as_str(), "storm-0001");
        assert_eq!(hive_name(999).as_str(), "storm-1000");
    }
}
