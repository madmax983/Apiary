//! How it works.

use autumn_web::AppState;
use autumn_web::prelude::*;

use super::layout::page;
use crate::Apiary;

/// The about page.
#[get("/about")]
#[public]
pub async fn show(State(state): State<AppState>) -> AutumnResult<Markup> {
    let apiary = Apiary::of(&state)?;
    let max_open = apiary.fleet.max_open();
    let grace = apiary.settings.grace.as_secs();
    Ok(page(
        "How it works",
        None,
        &html! {
            div class="prose" {
                h2 { "How it works" }
                p {
                    "SQLite has one writer per database. Give every tenant its own database and "
                    "that single process-wide writer becomes one writer per tenant; each tenant "
                    "also gets a file you can copy, restore or delete on its own. autumn-web 0.9 "
                    "calls this a database fleet. Apiary is a window onto one."
                }
                h3 { "The fleet" }
                p {
                    "The control database holds sessions and offboarding records. Every hive is "
                    "its own file under the fleet root, created only when somebody founds it — "
                    "walking into a hive that does not exist is a 404, not an empty file. A "
                    "hive's file is opened (and migrated) on first use and closed when it has sat "
                    "idle, or when the fleet already has " (max_open) " open and needs the room — "
                    "though never a file somebody used in the last two seconds, so a burst runs "
                    "past the cap and the sweeper brings it back. "
                    "Handlers do not know any of this: the notes page uses an ordinary generated "
                    "repository, declared " code { "#[repository(Note, tenant_scoped, sharded)]" }
                    ", and the search page runs the same repository over every file with "
                    code { "CrossShard" } "."
                }
                h3 { "The comb" }
                p {
                    "A lifecycle hook on the fleet reports every open and close. The map folds "
                    "them up and is pushed to every open comb at most four times a second over "
                    "server-sent events. The gauge reads the fleet's own counters, not the map."
                }
                h3 { "The storm" }
                p {
                    "A seeded workload over 1,000 hives, with 70% of touches on a hot set of 24: "
                    "more hives in play than the fleet will hold, which is the case its "
                    "least-recently-used closing exists for. The plan provisions each hive "
                    "before its first touch; " code { "verus/src/storm.rs" } " proves the plan "
                    "can never touch a hive it has not provisioned, nor provision one twice."
                }
                h3 { "Offboarding" }
                p {
                    "The only irreversible thing Apiary does is delete a hive's file, and it does "
                    "it only at the end of a durable workflow on the embedded SQLite Harvest "
                    "runtime: write a backup, check the backup on its own (integrity check, and "
                    "at least as many notes as the hive held), wait " (grace) " seconds for "
                    "anybody to say keep, then delete. Every outcome goes through one small state "
                    "machine, and " code { "verus/src/offboard.rs" } " proves that in every state "
                    "it can reach, a file may be deleted only after a verified backup, an elapsed "
                    "grace period, and no keep."
                }
            }
        },
    ))
}
