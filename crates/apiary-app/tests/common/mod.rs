//! Shared fixtures: a real `DatabaseFleet` in a temporary directory, migrated
//! with the app's own migrations, a control database beside it, and an app
//! around both.
#![allow(dead_code)]

use std::path::Path;
use std::time::Duration;

use apiary_app::offboard::Driver;
use apiary_app::routes;
use apiary_app::settings::Settings;
use autumn_web::config::{AutumnConfig, DatabaseConfig, DatabaseFleetConfig};
use autumn_web::db::fleet::DatabaseFleet;
use autumn_web::db::{RuntimeConnection, create_pool};
use autumn_web::fleet_layout::FleetMode;
use autumn_web::migrate::{EmbeddedMigrations, embed_migrations};
use autumn_web::prelude::*;
use autumn_web::reexports::{diesel, diesel_async};
use autumn_web::test::{TestApp, TestClient};
use diesel_async::RunQueryDsl as _;
use diesel_async::pooled_connection::deadpool::Pool;

/// The app's migrations, exactly as `main.rs` embeds them.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// A control database in `dir`, with the offboardings table.
pub async fn control_pool(dir: &Path) -> Pool<RuntimeConnection> {
    let pool = create_pool(&DatabaseConfig {
        url: Some(format!("sqlite://{}", dir.join("control.db").display())),
        ..Default::default()
    })
    .expect("control pool builds")
    .expect("url configured");
    let mut conn = pool.get().await.expect("control connection");
    for sql in [
        include_str!("../../migrations/2026-10-31-000001_create_notes/up.sql"),
        include_str!("../../migrations/2026-10-31-000002_create_offboardings/up.sql"),
    ] {
        diesel::sql_query(sql)
            .execute(&mut *conn)
            .await
            .expect("control migration");
    }
    pool
}

/// A tenant fleet under `dir/fleet` that keeps `max_open` files open.
pub fn fleet(dir: &Path, max_open: usize) -> DatabaseFleet {
    DatabaseFleet::builder(DatabaseFleetConfig {
        mode: FleetMode::Tenant,
        root: dir.join("fleet").display().to_string(),
        path: None,
        max_open,
        pool_size: 2,
        create_on_demand: Some(false),
        idle_close_secs: 0,
        restore_missing: false,
        ownership: None,
    })
    .migrations("app", MIGRATIONS)
    .min_residency(Duration::ZERO)
    .build()
    .expect("fleet builds")
}

/// Header tenancy, no CSRF: the shape the framework's own fleet test uses.
pub fn config() -> AutumnConfig {
    let mut config = AutumnConfig {
        profile: Some("test".into()),
        ..AutumnConfig::default()
    };
    config.security.csrf.enabled = false;
    config.tenancy.enabled = true;
    "header".clone_into(&mut config.tenancy.source);
    "x-tenant-id".clone_into(&mut config.tenancy.header_name);
    config.tenancy.public_paths = [
        "/",
        "/_map",
        "/stream",
        "/hives",
        "/enter",
        "/storm",
        "/search",
        "/about",
        "/apiary.js",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    config.health.detailed = true;
    config
}

/// Settings pointing the archive into `dir`.
pub fn settings(dir: &Path, grace_secs: u64) -> Settings {
    let archive = dir.join("archive").display().to_string();
    let harvest = dir.join("harvest.db").display().to_string();
    Settings::from_lookup(|k| match k {
        "APIARY_GRACE_SECS" => Some(grace_secs.to_string()),
        "APIARY_ARCHIVE_DIR" => Some(archive.clone()),
        "APIARY_HARVEST_DB" => Some(harvest.clone()),
        _ => None,
    })
}

/// The whole app, around `fleet`, with an in-memory Harvest runtime.
pub fn app(
    control: Pool<RuntimeConnection>,
    fleet: DatabaseFleet,
    settings: Settings,
) -> TestClient {
    TestApp::new()
        .config(config())
        .routes(routes![
            routes::comb::index,
            routes::comb::map_fragment,
            routes::comb::found,
            routes::comb::enter,
            routes::comb::raise_storm,
            routes::hive::index,
            routes::hive::notes,
            routes::hive::add_note,
            routes::hive::export,
            routes::hive::offboard,
            routes::hive::keep,
            routes::hive::restore,
            routes::hive::offboarding,
            routes::search::index,
            routes::about::show,
            routes::assets::script,
        ])
        .with_db(control)
        .with_fleet(fleet)
        .state_initializer(move |state| {
            let driver = Driver::spawn_in_memory().expect("in-memory harvest");
            apiary_app::install(state, settings, driver).expect("apiary installs");
        })
        .build()
}
