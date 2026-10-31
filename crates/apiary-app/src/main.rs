// Apiary — one SQLite hive per tenant, and a window onto the fleet.
//
//   Engine       -> hive: hive names, the offboarding state machine, the storm
//                   plan and the fleet map. No IO, no async, no clock.
//   Fleet        -> autumn-web 0.9's DatabaseFleet: one SQLite file per tenant
//                   under `apiary-fleet/`, opened and migrated on first use,
//                   closed when idle or least recently used past max_open.
//   Control      -> one SQLite file for sessions and offboarding rows.
//   Workflows    -> autumn-harvest-sqlite: the offboarding runs durably in its
//                   own SQLite file, single-writer, driven by one task.
//   Live events  -> one SSE topic; the comb redraws on it.
//
// Run with:  cargo run -p apiary-app
// Then open: http://localhost:3000

use apiary_app::routes;
use autumn_web::migrate::{EmbeddedMigrations, embed_migrations};
use autumn_web::prelude::*;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

#[autumn_web::main]
async fn main() {
    // Boxed: the server future is ~23 KB, too big to keep on the stack.
    Box::pin(
        autumn_web::app()
            // One set, applied to the control database AND to every hive file:
            // autumn-web 0.9 has no control-only app set. See
            // docs/adr/0002-one-migration-set.md.
            .migrations(MIGRATIONS)
            .on_startup(apiary_app::boot)
            .routes(routes![
                routes::comb::index,
                routes::comb::map_fragment,
                routes::comb::stream,
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
            .run(),
    )
    .await;
}
