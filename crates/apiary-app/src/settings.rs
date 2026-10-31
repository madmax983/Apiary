//! Apiary's own settings.
//!
//! `autumn.toml` refuses keys it does not know, so these come from the
//! environment, each with a default that works under `cargo run`.

use std::path::PathBuf;
use std::time::Duration;

/// Settings read once at boot.
#[derive(Debug, Clone)]
pub struct Settings {
    /// How long an offboarded hive waits for a keep before it is deleted.
    /// `APIARY_GRACE_SECS`, default 90.
    pub grace: Duration,
    /// Where verified backups go. `APIARY_ARCHIVE_DIR`, default
    /// `apiary-archive`.
    pub archive_dir: PathBuf,
    /// The Harvest runtime's own `SQLite` file. `APIARY_HARVEST_DB`, default
    /// `apiary-harvest.db`.
    pub harvest_db: PathBuf,
}

impl Settings {
    /// Read from the process environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Read from any lookup, for tests.
    #[must_use]
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let grace = get("APIARY_GRACE_SECS")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|s| (1..=30 * 24 * 3600).contains(s))
            .unwrap_or(90);
        Self {
            grace: Duration::from_secs(grace),
            archive_dir: get("APIARY_ARCHIVE_DIR")
                .filter(|v| !v.trim().is_empty())
                .map_or_else(|| PathBuf::from("apiary-archive"), PathBuf::from),
            harvest_db: get("APIARY_HARVEST_DB")
                .filter(|v| !v.trim().is_empty())
                .map_or_else(|| PathBuf::from("apiary-harvest.db"), PathBuf::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_work_under_cargo_run() {
        let s = Settings::from_lookup(|_| None);
        assert_eq!(s.grace, Duration::from_secs(90));
        assert_eq!(s.archive_dir, PathBuf::from("apiary-archive"));
        assert_eq!(s.harvest_db, PathBuf::from("apiary-harvest.db"));
    }

    #[test]
    fn a_nonsense_grace_falls_back() {
        for bad in ["", "0", "-5", "soon", "999999999"] {
            let s = Settings::from_lookup(|k| (k == "APIARY_GRACE_SECS").then(|| bad.to_owned()));
            assert_eq!(s.grace, Duration::from_secs(90), "{bad}");
        }
        let s = Settings::from_lookup(|k| (k == "APIARY_GRACE_SECS").then(|| " 5 ".to_owned()));
        assert_eq!(s.grace, Duration::from_secs(5));
    }
}
