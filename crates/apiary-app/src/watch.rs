//! The window onto the fleet.
//!
//! A [`FleetLifecycle`] hook on the `DatabaseFleet` reports every open and
//! close of a hive's file; [`FleetWatch`] folds them into a
//! [`hive::FleetMap`] and marks the map dirty. A ticker publishes one message
//! on [`TOPIC`] at most every [`TICK`] while anything changed, and every open
//! front page refetches its map on it. A storm makes thousands of opens a
//! second; the browsers see four redraws.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use autumn_web::db::fleet::{DatabaseFleet, FleetDatabase, FleetLifecycle};
use autumn_web::fleet_layout::FleetDbKey;
use hive::{FleetMap, HiveName, MapEvent};

/// The live-events topic the map redraws on.
pub const TOPIC: &str = "fleet";

/// How often, at most, the map is pushed to browsers.
pub const TICK: Duration = Duration::from_millis(250);

/// The map, shared by the hook, the ticker and the handlers.
#[derive(Debug)]
pub struct FleetWatch {
    map: Mutex<FleetMap>,
    dirty: AtomicBool,
    epoch: Instant,
    restores: Mutex<HashMap<String, PathBuf>>,
}

impl Default for FleetWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl FleetWatch {
    /// An empty watch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: Mutex::new(FleetMap::new()),
            dirty: AtomicBool::new(false),
            epoch: Instant::now(),
            restores: Mutex::new(HashMap::new()),
        }
    }

    /// Arrange for the next open of `db_name` (a missing file) to be restored
    /// from `backup`. The fleet asks [`Observer::restore_missing`] under its
    /// own path lock, so nothing else publishes that file meanwhile.
    pub fn stage_restore(&self, db_name: String, backup: PathBuf) {
        self.restores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(db_name, backup);
    }

    /// Drop a staged restore, whether or not it was used.
    pub fn unstage_restore(&self, db_name: &str) -> Option<PathBuf> {
        self.restores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(db_name)
    }

    fn staged(&self, db_name: &str) -> Option<PathBuf> {
        self.restores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(db_name)
            .cloned()
    }

    /// Milliseconds since the watch started (the map's clock).
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Fold an event in.
    pub fn record(&self, event: MapEvent) {
        let at = self.now_ms().max(1);
        self.lock().apply(event, at);
        self.dirty.store(true, Ordering::Release);
    }

    /// Fold an event in by fleet database name (`tenant:acme`). Names that
    /// are not hive names (a slot database, should one appear) are ignored.
    pub fn record_db(&self, db_name: &str, make: fn(HiveName) -> MapEvent) {
        if let Some(name) = HiveName::from_db_name(db_name) {
            self.record(make(name));
        }
    }

    /// A copy of the map, for drawing.
    #[must_use]
    pub fn snapshot(&self) -> FleetMap {
        self.lock().clone()
    }

    /// Whether anything changed since the last call; clears the flag.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    /// The map lock. A panic while holding it can only have left a partly
    /// folded picture, which the next event repairs, so a poisoned lock is
    /// used rather than propagated.
    fn lock(&self) -> MutexGuard<'_, FleetMap> {
        self.map.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Put every hive already on disk on the map, as a cold tile.
    ///
    /// # Errors
    ///
    /// The fleet's error listing its root.
    pub async fn seed(
        &self,
        fleet: &DatabaseFleet,
    ) -> Result<usize, autumn_web::db::fleet::FleetError> {
        let keys = fleet.list().await?;
        let mut n = 0;
        for key in keys {
            if let Some(name) = HiveName::from_db_name(&key.name()) {
                self.record(MapEvent::Known(name));
                n += 1;
            }
        }
        Ok(n)
    }
}

/// The lifecycle hook that feeds a [`FleetWatch`].
#[derive(Debug, Clone)]
pub struct Observer(pub Arc<FleetWatch>);

impl FleetLifecycle for Observer {
    /// Put a staged backup in place of a missing file. Runs on a blocking
    /// thread, under the fleet's path lock, before the fleet checks the file's
    /// identity row and migrates it — so a backup of another hive is refused
    /// by the fleet, not by this code.
    fn restore_missing(&self, key: &FleetDbKey, path: &Path) -> Result<bool, String> {
        let Some(backup) = self.0.staged(&key.name()) else {
            return Ok(false);
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        // Copy to a sibling, then rename: the fleet never sees half a file.
        let partial = path.with_extension("db.restoring");
        std::fs::copy(&backup, &partial).map_err(|e| format!("{}: {e}", backup.display()))?;
        std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(true)
    }

    fn on_open(&self, db: &FleetDatabase) -> Result<(), String> {
        self.0.record_db(&db.key().name(), MapEvent::Opened);
        Ok(())
    }

    fn on_close(&self, db: &FleetDatabase) {
        self.0.record_db(&db.key().name(), MapEvent::Closed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_by_db_name_and_ignores_slots() {
        let w = FleetWatch::new();
        w.record_db("tenant:acme", MapEvent::Opened);
        w.record_db("slot:00042", MapEvent::Opened);
        w.record_db("tenant:Not-A-Hive", MapEvent::Opened);
        let m = w.snapshot();
        assert_eq!(m.len(), 1);
        assert_eq!(m.open_now(), 1);
    }

    #[test]
    fn restores_are_staged_and_unstaged_by_db_name() {
        let w = FleetWatch::new();
        assert!(w.staged("tenant:acme").is_none());
        w.stage_restore("tenant:acme".into(), PathBuf::from("/b/acme.db"));
        assert_eq!(w.staged("tenant:acme"), Some(PathBuf::from("/b/acme.db")));
        assert_eq!(
            w.unstage_restore("tenant:acme"),
            Some(PathBuf::from("/b/acme.db"))
        );
        assert!(w.staged("tenant:acme").is_none());
    }

    #[test]
    fn dirty_is_taken_once() {
        let w = FleetWatch::new();
        assert!(!w.take_dirty());
        w.record_db("tenant:acme", MapEvent::Opened);
        w.record_db("tenant:acme", MapEvent::Closed);
        assert!(w.take_dirty());
        assert!(!w.take_dirty());
    }
}
