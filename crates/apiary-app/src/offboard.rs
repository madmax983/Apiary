//! Offboarding a hive: a durable Harvest workflow on the embedded `SQLite`
//! backend (`autumn-harvest-sqlite`).
//!
//! ```text
//!  begin ─► snapshot_hive ─► verify_snapshot ─┬─ failed ─► aborted (nothing deleted)
//!   (record each phase)                       └─ ok ─► grace: wait for "keep", or the timer
//!                                                         ├─ keep  ─► kept
//!                                                         └─ timer ─► purge_hive ─► deleted
//! ```
//!
//! Every outcome goes through [`hive::Offboarding::apply`] — the machine whose
//! safety is proved in `verus/src/offboard.rs` — and `purge_hive` is only
//! scheduled when [`hive::Offboarding::may_purge`] says so. A restart in the
//! middle replays the history and carries on from the last recorded step; the
//! grace timer is durable, so it survives a restart too.
//!
//! The runtime is single-writer, so one task owns it: [`Driver`]. Handlers
//! talk to it through a channel; it ticks every [`DRIVE_EVERY`] to fire timers
//! that have come due.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use autumn_harvest::prelude::*;
use autumn_harvest_sqlite::SqliteRuntime;
use autumn_web::db::RuntimeConnection;
use autumn_web::db::fleet::DatabaseFleet;
use autumn_web::reexports::{diesel, diesel_async};
use diesel::{
    ExpressionMethods as _, OptionalExtension as _, QueryDsl as _, SelectableHelper as _,
};
use diesel_async::RunQueryDsl as _;
use diesel_async::pooled_connection::deadpool::Pool;
use hive::{HiveName, Offboarding, Phase, Step};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::models::OffboardingRow;
use crate::schema::offboardings;
use crate::watch::FleetWatch;

/// The workflow's registered name.
pub const WORKFLOW: &str = "offboard_hive";
/// The signal that keeps a hive.
pub const KEEP: &str = "keep";
/// How often the driver fires due timers when nothing else wakes it.
pub const DRIVE_EVERY: Duration = Duration::from_secs(1);

/// What the activities need. Activities are free functions the runtime calls
/// by name, so this is installed once at boot rather than passed in.
#[derive(Clone)]
pub struct Deps {
    /// The hive fleet.
    pub fleet: DatabaseFleet,
    /// The control database, where offboarding rows live.
    pub control: Pool<RuntimeConnection>,
    /// Where backups are written.
    pub archive_dir: PathBuf,
    /// The map, told when a hive is deleted.
    pub watch: std::sync::Arc<FleetWatch>,
}

impl std::fmt::Debug for Deps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps")
            .field("fleet_root", &self.fleet.root())
            .field("archive_dir", &self.archive_dir)
            .finish_non_exhaustive()
    }
}

static DEPS: OnceLock<Deps> = OnceLock::new();

/// Install the activities' dependencies. `false` if already installed (a
/// second app in one process, as in tests, keeps the first).
pub fn install(deps: Deps) -> bool {
    DEPS.set(deps).is_ok()
}

fn deps() -> Result<&'static Deps, String> {
    DEPS.get()
        .ok_or_else(|| "offboarding dependencies are not installed".to_owned())
}

/// The workflow's input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffboardInput {
    /// The hive.
    pub hive: String,
    /// The workflow id, so activities can stamp rows with it.
    pub workflow_id: String,
    /// Grace period in seconds.
    pub grace_secs: u64,
}

/// A backup, as `snapshot_hive` made it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// The hive.
    pub hive: String,
    /// The backup file.
    pub path: String,
    /// Notes in the live file, counted just before the backup was taken.
    pub rows_before: i64,
}

/// One phase change, for `record_phase`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRecord {
    /// The hive.
    pub hive: String,
    /// The workflow.
    pub workflow_id: String,
    /// The phase reached.
    pub phase: Phase,
    /// One line about it.
    pub detail: String,
    /// Seconds of grace from now, when entering `Grace`.
    pub grace_secs: Option<u64>,
    /// The verified backup, from `Grace` on.
    pub backup: Option<String>,
}

fn stepped(o: Offboarding, step: Step) -> Result<Offboarding, String> {
    o.apply(step).map_err(|e| e.to_string())
}

async fn record(
    ctx: &WorkflowContext,
    input: &OffboardInput,
    o: Offboarding,
    detail: String,
    grace_secs: Option<u64>,
    backup: Option<&Snapshot>,
) -> Result<(), String> {
    let rec = PhaseRecord {
        hive: input.hive.clone(),
        workflow_id: input.workflow_id.clone(),
        phase: o.phase(),
        detail,
        grace_secs,
        // Only a verified backup is ever offered for a restore.
        backup: backup.filter(|_| o.verified()).map(|s| s.path.clone()),
    };
    ctx.execute_activity::<_, ()>(&record_phase_info(), rec)
        .await
        .map_err(|e| e.to_string())
}

/// Offboard one hive.
#[workflow]
pub async fn offboard_hive(ctx: &WorkflowContext, input: OffboardInput) -> Result<String, String> {
    let mut o = stepped(Offboarding::new(), Step::Begin)?;
    record(ctx, &input, o, "writing a backup".into(), None, None).await?;

    let snap: Snapshot = ctx
        .execute_activity(&snapshot_hive_info(), input.hive.clone())
        .await
        .map_err(|e| e.to_string())?;
    o = stepped(o, Step::SnapshotTaken)?;
    record(
        ctx,
        &input,
        o,
        format!("backup at {} — checking it", snap.path),
        None,
        None,
    )
    .await?;

    let check: Result<(), String> = ctx
        .execute_activity(&verify_snapshot_info(), snap.clone())
        .await
        .map_err(|e| e.to_string())?;
    o = stepped(o, Step::SnapshotVerified { ok: check.is_ok() })?;
    if let Err(why) = check {
        record(
            ctx,
            &input,
            o,
            format!("backup failed its check: {why}"),
            None,
            None,
        )
        .await?;
        return Ok("aborted".into());
    }
    record(
        ctx,
        &input,
        o,
        format!(
            "backup verified ({} notes) at {}",
            snap.rows_before, snap.path
        ),
        Some(input.grace_secs),
        Some(&snap),
    )
    .await?;

    let keep = ctx
        .wait_for_signal_timeout(KEEP, Duration::from_secs(input.grace_secs))
        .await
        .map_err(|e| e.to_string())?;
    if keep.is_some() {
        o = stepped(o, Step::Keep)?;
        record(
            ctx,
            &input,
            o,
            "kept — nothing was deleted".into(),
            None,
            Some(&snap),
        )
        .await?;
        return Ok("kept".into());
    }
    o = stepped(o, Step::GraceElapsed)?;
    record(
        ctx,
        &input,
        o,
        "grace elapsed — deleting the file".into(),
        None,
        Some(&snap),
    )
    .await?;

    // The gate. `may_purge` is `phase == Purging`, and the Verus theorem says
    // a Purging state has a verified backup, an elapsed grace and no keep.
    if !o.may_purge() {
        return Err(format!("refusing to purge from {:?}", o.phase()));
    }
    ctx.execute_activity::<_, bool>(&purge_hive_info(), input.hive.clone())
        .await
        .map_err(|e| e.to_string())?;
    o = stepped(o, Step::Purged)?;
    record(
        ctx,
        &input,
        o,
        format!("deleted; restorable from {}", snap.path),
        None,
        Some(&snap),
    )
    .await?;
    Ok("deleted".into())
}

fn hive_key(
    fleet: &DatabaseFleet,
    hive: &str,
) -> Result<autumn_web::fleet_layout::FleetDbKey, String> {
    let name = HiveName::parse(hive).map_err(|e| e.to_string())?;
    fleet.key_for(name.as_str()).map_err(|e| e.to_string())
}

async fn count_notes(fleet: &DatabaseFleet, hive: &str) -> Result<i64, String> {
    let key = hive_key(fleet, hive)?;
    let db = fleet.open(&key).await.map_err(|e| e.to_string())?;
    let _lease = db.lease();
    let mut conn = db.pool().get().await.map_err(|e| e.to_string())?;
    let c: crate::models::Count = diesel::sql_query("SELECT COUNT(*) AS n FROM notes")
        .get_result(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(c.n)
}

/// Write the phase to the control database.
#[activity]
pub async fn record_phase(_ctx: &ActivityContext, rec: PhaseRecord) -> Result<(), String> {
    let d = deps()?;
    let now = chrono::Utc::now().naive_utc();
    let grace_until = rec
        .grace_secs
        .and_then(|s| chrono::Duration::try_seconds(i64::try_from(s).ok()?))
        .map(|g| now + g);
    let row = OffboardingRow {
        hive: rec.hive,
        workflow_id: rec.workflow_id,
        phase: rec.phase.label().to_owned(),
        detail: rec.detail,
        backup: rec.backup,
        grace_until,
        updated_at: now,
    };
    let mut conn = d.control.get().await.map_err(|e| e.to_string())?;
    diesel::replace_into(offboardings::table)
        .values(&row)
        .execute(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Count the hive's notes, then write a backup of its file to the archive.
#[activity]
pub async fn snapshot_hive(_ctx: &ActivityContext, hive: String) -> Result<Snapshot, String> {
    let d = deps()?;
    let key = hive_key(&d.fleet, &hive)?;
    let rows_before = count_notes(&d.fleet, &hive).await?;
    std::fs::create_dir_all(&d.archive_dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3f");
    let path = d.archive_dir.join(format!("{hive}-{stamp}.db"));
    d.fleet
        .backup(&key, &path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Snapshot {
        hive,
        path: path.display().to_string(),
        rows_before,
    })
}

/// Check a backup on its own.
///
/// It must open, pass `PRAGMA integrity_check`, and hold at least the notes
/// the live file held before it was taken (notes are append-only, so a sound
/// backup can only hold more). The outer `Result` is the activity failing;
/// the inner one is the verdict.
#[activity]
pub async fn verify_snapshot(
    _ctx: &ActivityContext,
    snap: Snapshot,
) -> Result<Result<(), String>, String> {
    let path = snap.path.clone();
    tokio::task::spawn_blocking(move || Ok(check_backup(&path, snap.rows_before)))
        .await
        .map_err(|e| e.to_string())?
}

pub use backup_check::check_backup;

/// The backup check runs on a blocking thread with a plain (sync) Diesel
/// connection, so it lives apart from the async `RunQueryDsl` this module uses.
mod backup_check {
    use diesel::{Connection as _, QueryableByName, RunQueryDsl as _};

    /// The check `verify_snapshot` runs, on a blocking thread.
    ///
    /// # Errors
    ///
    /// Why the backup is not good enough.
    pub fn check_backup(path: &str, rows_before: i64) -> Result<(), String> {
        #[derive(QueryableByName)]
        struct Integrity {
            #[diesel(sql_type = diesel::sql_types::Text)]
            integrity_check: String,
        }

        if !std::path::Path::new(path).is_file() {
            return Err(format!("{path} does not exist"));
        }
        let url = format!("file:{path}?mode=ro");
        let mut conn = diesel::SqliteConnection::establish(&url).map_err(|e| e.to_string())?;
        let rows: Vec<Integrity> = diesel::sql_query("PRAGMA integrity_check")
            .load(&mut conn)
            .map_err(|e| e.to_string())?;
        if rows.len() != 1 || rows[0].integrity_check != "ok" {
            let said: Vec<String> = rows.into_iter().map(|r| r.integrity_check).collect();
            return Err(format!("integrity_check: {}", said.join("; ")));
        }
        let n: crate::models::Count = diesel::sql_query("SELECT COUNT(*) AS n FROM notes")
            .get_result(&mut conn)
            .map_err(|e| e.to_string())?;
        if n.n < rows_before {
            return Err(format!(
                "backup holds {} notes, the hive held {rows_before}",
                n.n
            ));
        }
        Ok(())
    }
}

/// Delete the hive's database file. Only ever scheduled behind `may_purge`.
#[activity]
pub async fn purge_hive(_ctx: &ActivityContext, hive: String) -> Result<bool, String> {
    let d = deps()?;
    let key = hive_key(&d.fleet, &hive)?;
    let gone = d.fleet.delete(&key).await.map_err(|e| e.to_string())?;
    if let Ok(name) = HiveName::parse(&hive) {
        d.watch.record(hive::MapEvent::Deleted(name));
    }
    Ok(gone)
}

/// Why the driver refused a command.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// The driver task is gone.
    #[error("the offboarding driver is not running")]
    Gone,
    /// The runtime refused.
    #[error("{0}")]
    Runtime(String),
}

enum Cmd {
    Start {
        hive: HiveName,
        grace_secs: u64,
        reply: oneshot::Sender<Result<String, String>>,
    },
    Keep {
        workflow_id: String,
        reply: oneshot::Sender<Result<bool, String>>,
    },
}

/// A handle on the task that owns the Harvest runtime.
#[derive(Debug, Clone)]
pub struct Driver {
    tx: mpsc::Sender<Cmd>,
}

impl std::fmt::Debug for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start { hive, .. } => write!(f, "Start({hive})"),
            Self::Keep { workflow_id, .. } => write!(f, "Keep({workflow_id})"),
        }
    }
}

/// The runtime with Apiary's workflow and activities registered.
fn registered(mut rt: SqliteRuntime) -> SqliteRuntime {
    rt.register_workflow(&offboard_hive_info());
    rt.register_activity(&record_phase_info());
    rt.register_activity(&snapshot_hive_info());
    rt.register_activity(&verify_snapshot_info());
    rt.register_activity(&purge_hive_info());
    rt
}

impl Driver {
    /// Open (or create) the Harvest file and start the task that owns it.
    ///
    /// # Errors
    ///
    /// The runtime could not open its file — most likely another process
    /// holds its lock.
    pub fn spawn(path: &std::path::Path) -> Result<Self, DriverError> {
        let rt = SqliteRuntime::open(path).map_err(|e| DriverError::Runtime(e.to_string()))?;
        Ok(Self::spawn_runtime(registered(rt)))
    }

    /// Start the task on an in-memory runtime, for tests.
    ///
    /// # Errors
    ///
    /// The runtime could not be created.
    pub fn spawn_in_memory() -> Result<Self, DriverError> {
        let rt =
            SqliteRuntime::open_in_memory().map_err(|e| DriverError::Runtime(e.to_string()))?;
        Ok(Self::spawn_runtime(registered(rt)))
    }

    fn spawn_runtime(mut rt: SqliteRuntime) -> Self {
        let (tx, mut rx) = mpsc::channel::<Cmd>(64);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(DRIVE_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    cmd = rx.recv() => match cmd {
                        Some(cmd) => handle(&mut rt, cmd),
                        None => break,
                    },
                    _ = tick.tick() => {}
                }
                if let Err(e) = rt.run_until_idle().await {
                    tracing::warn!(error = %e, "offboarding driver pass failed");
                }
            }
        });
        Self { tx }
    }

    /// Start offboarding `hive`. Returns the workflow id.
    ///
    /// # Errors
    ///
    /// The driver is gone, or the runtime refused the start.
    pub async fn start(&self, hive: HiveName, grace: Duration) -> Result<String, DriverError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Start {
                hive,
                grace_secs: grace.as_secs().max(1),
                reply,
            })
            .await
            .map_err(|_| DriverError::Gone)?;
        rx.await
            .map_err(|_| DriverError::Gone)?
            .map_err(DriverError::Runtime)
    }

    /// Send the keep signal to an offboarding. `Ok(false)` when it was a
    /// repeat.
    ///
    /// # Errors
    ///
    /// The driver is gone, or the runtime refused (the run already ended).
    pub async fn keep(&self, workflow_id: String) -> Result<bool, DriverError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Keep { workflow_id, reply })
            .await
            .map_err(|_| DriverError::Gone)?;
        rx.await
            .map_err(|_| DriverError::Gone)?
            .map_err(DriverError::Runtime)
    }
}

fn handle(rt: &mut SqliteRuntime, cmd: Cmd) {
    match cmd {
        Cmd::Start {
            hive,
            grace_secs,
            reply,
        } => {
            let workflow_id = format!("offboard:{hive}:{}", chrono::Utc::now().timestamp_millis());
            let input = OffboardInput {
                hive: hive.to_string(),
                workflow_id: workflow_id.clone(),
                grace_secs,
            };
            let result = serde_json::to_value(&input)
                .map_err(|e| e.to_string())
                .and_then(|v| {
                    rt.start_workflow_with_id(WORKFLOW, &workflow_id, v)
                        .map_err(|e| e.to_string())
                })
                .map(|_| workflow_id);
            let _ = reply.send(result);
        }
        Cmd::Keep { workflow_id, reply } => {
            let result = rt
                .send_signal_by_workflow_id(
                    WORKFLOW,
                    &workflow_id,
                    KEEP,
                    serde_json::json!({ "kept": true }),
                    Some("keep"),
                )
                .map(|d| d.delivered)
                .map_err(|e| e.to_string());
            let _ = reply.send(result);
        }
    }
}

/// The offboarding row for `hive`, if it has ever been offboarded.
///
/// # Errors
///
/// The control database failed.
pub async fn row_for(
    conn: &mut RuntimeConnection,
    hive: &str,
) -> Result<Option<OffboardingRow>, diesel::result::Error> {
    offboardings::table
        .filter(offboardings::hive.eq(hive))
        .select(OffboardingRow::as_select())
        .first(conn)
        .await
        .optional()
}

/// Record that a deleted hive was put back from `backup`.
///
/// # Errors
///
/// The control database failed.
pub async fn mark_restored(
    conn: &mut RuntimeConnection,
    row: &OffboardingRow,
    backup: &str,
) -> Result<(), diesel::result::Error> {
    let now = chrono::Utc::now().naive_utc();
    diesel::replace_into(offboardings::table)
        .values(&OffboardingRow {
            phase: "restored".to_owned(),
            detail: format!("restored from {backup}"),
            grace_until: None,
            updated_at: now,
            ..row.clone()
        })
        .execute(conn)
        .await
        .map(|_| ())
}

/// Every offboarding, newest first.
///
/// # Errors
///
/// The control database failed.
pub async fn all_rows(
    conn: &mut RuntimeConnection,
) -> Result<Vec<OffboardingRow>, diesel::result::Error> {
    offboardings::table
        .order(offboardings::updated_at.desc())
        .select(OffboardingRow::as_select())
        .limit(50)
        .load(conn)
        .await
}

/// Whether an offboarding in `phase` is still running (so the page polls).
#[must_use]
pub fn in_flight(phase: &str) -> bool {
    matches!(phase, "snapshotting" | "verifying" | "grace" | "purging")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_flight_matches_the_non_terminal_phases() {
        for p in [
            Phase::Active,
            Phase::Snapshotting,
            Phase::Verifying,
            Phase::Grace,
            Phase::Kept,
            Phase::Aborted,
            Phase::Purging,
            Phase::Deleted,
        ] {
            assert_eq!(
                in_flight(p.label()),
                !p.is_terminal() && p != Phase::Active,
                "{p:?}"
            );
        }
    }

    #[test]
    fn a_missing_backup_fails_the_check() {
        let err = check_backup("/nonexistent/backup.db", 0).unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
    }

    #[test]
    fn a_garbage_backup_fails_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("junk.db");
        std::fs::write(&p, b"this is not a sqlite database at all, not even close").unwrap();
        assert!(check_backup(p.to_str().unwrap(), 0).is_err());
    }
}
