//! Apiary's web application.
//!
//! The rules live in the `hive` crate; this crate knows about HTTP, the fleet,
//! the control database and the Harvest runtime.
//!
//! ```text
//!   browser ──► routes ──► ShardedDb / PgNoteRepository ──► the request's hive file
//!                  │                CrossShard ──► every hive file, in turn
//!                  ├──► storm ──► DatabaseFleet (open / provision / LRU)
//!                  ├──► offboard::Driver ──► SqliteRuntime (apiary-harvest.db)
//!                  │                              └─ activities ──► fleet.backup / delete
//!                  └──► control database (sessions, offboarding rows)
//!   DatabaseFleet ──on_open/on_close──► watch::Observer ──► FleetMap ──tick──► SSE ──► every comb
//! ```
#![warn(missing_docs)]

// `#[autumn_web::model]` and `#[repository]` generate a `NewNote`, an update
// struct, a column enum and finder methods, and document none of them.
#[allow(missing_docs)]
pub mod models;
pub mod offboard;
pub mod routes;
// `diesel::table!` documents none of what it generates.
#[allow(missing_docs)]
pub mod schema;
pub mod settings;
pub mod storm;
pub mod watch;

use std::sync::Arc;

use autumn_web::AppState;
use autumn_web::db::fleet::DatabaseFleet;
use autumn_web::error::AutumnError;

use crate::offboard::{Deps, Driver};
use crate::settings::Settings;
use crate::storm::StormState;
use crate::watch::{FleetWatch, Observer, TICK, TOPIC};

/// Everything Apiary hangs on the `AppState`.
#[derive(Debug, Clone)]
pub struct Apiary {
    /// The fleet of hive files.
    pub fleet: DatabaseFleet,
    /// The live picture of it.
    pub watch: Arc<FleetWatch>,
    /// The storm button's state.
    pub storm: Arc<StormState>,
    /// The offboarding runtime.
    pub driver: Driver,
    /// Settings from the environment.
    pub settings: Settings,
}

impl Apiary {
    /// Apiary's state, from a handler.
    ///
    /// # Errors
    ///
    /// `503` before boot finished, or with no fleet configured.
    pub fn of(state: &AppState) -> Result<Arc<Self>, AutumnError> {
        state.extension::<Self>().ok_or_else(|| {
            AutumnError::service_unavailable_msg(
                "Apiary is not ready: no [database.fleet] configured, or boot has not finished",
            )
        })
    }
}

/// The startup hook: [`install`] Apiary, then put every hive already on disk
/// on the map.
///
/// # Errors
///
/// No fleet is configured, or the Harvest file is locked by another process.
pub async fn boot(state: AppState) -> autumn_web::AutumnResult<()> {
    let settings = Settings::from_env();
    let driver = Driver::spawn(&settings.harvest_db)
        .map_err(|e| AutumnError::service_unavailable_msg(e.to_string()))?;
    let apiary = install(&state, settings, driver)?;
    match apiary.watch.seed(&apiary.fleet).await {
        Ok(n) => {
            tracing::info!(hives = n, root = %apiary.fleet.root().display(), "the comb is seeded");
        }
        Err(e) => tracing::warn!(error = %e, "could not list the fleet root"),
    }
    Ok(())
}

/// Hang Apiary on `state`.
///
/// That is: the map hook on the fleet, the activities' dependencies, the
/// ticker, and the [`Apiary`] extension. Synchronous, so a test can call it
/// from `TestApp::state_initializer`; it must run inside a Tokio runtime (it
/// spawns the ticker).
///
/// # Errors
///
/// No fleet, or no control database, is configured.
pub fn install(
    state: &AppState,
    settings: Settings,
    driver: Driver,
) -> Result<Arc<Apiary>, AutumnError> {
    let fleet = state
        .shards()
        .and_then(|s| s.fleet())
        .cloned()
        .ok_or_else(|| {
            AutumnError::service_unavailable_msg(
                "Apiary needs a [database.fleet] section in autumn.toml",
            )
        })?;
    let control = state.pool().cloned().ok_or_else(|| {
        AutumnError::service_unavailable_msg("Apiary needs a control database ([database] url)")
    })?;

    let watch = Arc::new(FleetWatch::new());
    fleet.add_lifecycle(Arc::new(Observer(Arc::clone(&watch))));

    // A second app in one process (a test binary) keeps the first app's
    // dependencies; activities only ever run on the first app's driver there.
    let _ = offboard::install(Deps {
        fleet: fleet.clone(),
        control,
        archive_dir: settings.archive_dir.clone(),
        watch: Arc::clone(&watch),
    });

    let channels = state.channels().clone();
    let ticker_watch = Arc::clone(&watch);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if ticker_watch.take_dirty() {
                // No subscribers is not an error worth logging.
                let _ = channels.publish(TOPIC, "tick");
            }
        }
    });

    state.insert_extension(Apiary {
        fleet,
        watch,
        storm: Arc::new(StormState::default()),
        driver,
        settings,
    });
    Apiary::of(state)
}

#[cfg(test)]
mod tests {
    /// Every route the app declares is registered in `main.rs`; a route that
    /// is written but not registered fails silently as a 404.
    #[test]
    fn every_declared_route_is_registered() {
        let main_source = include_str!("main.rs");
        for handler in crate::routes::ALL {
            assert!(main_source.contains(handler), "{handler} is not registered");
        }
    }

    /// The store is files, and stays files. A Postgres URL in the dev profile
    /// would silently make `verify.sh` need a server that is not there.
    #[test]
    fn the_dev_profile_is_sqlite_with_a_tenant_fleet() {
        let dev = include_str!("../../../autumn-dev.toml");
        assert!(dev.contains("sqlite://"));
        assert!(!dev.contains("postgres://"));
        let base = include_str!("../../../autumn.toml");
        assert!(base.contains("[database.fleet]"));
        assert!(base.contains(r#"mode = "tenant""#));
        assert!(base.contains("create_on_demand = false"));
    }
}
