//! Searching every hive at once.
//!
//! `CrossShard<PgNoteRepository>` runs the same repository over every hive
//! file on disk — far more files than the fleet keeps open — so a search opens
//! them a few at a time, and the comb lights up in a wave as it goes.

use std::collections::BTreeMap;
use std::time::Instant;

use autumn_web::prelude::*;
use autumn_web::sharding::CrossShard;
use serde::Deserialize;

use super::layout::{commas, page};
use crate::models::{NoteRepository as _, PgNoteRepository};

/// The query string.
#[derive(Debug, Deserialize, Default)]
pub struct SearchQuery {
    /// What to look for.
    #[serde(default)]
    pub q: String,
}

/// Hits shown per hive.
pub const PER_HIVE: usize = 3;

/// The search page.
#[get("/search")]
#[public]
pub async fn index(
    CrossShard(repo): CrossShard<PgNoteRepository>,
    Query(query): Query<SearchQuery>,
) -> AutumnResult<Markup> {
    let q = query.q.trim().to_lowercase();
    let results = if q.is_empty() {
        None
    } else {
        let started = Instant::now();
        let all = repo.find_all().await?;
        let scanned = all.len();
        let mut by_hive: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for n in all {
            if n.body.to_lowercase().contains(&q) {
                by_hive
                    .entry(n.tenant_id.unwrap_or_default())
                    .or_default()
                    .push(n.body);
            }
        }
        Some((by_hive, scanned, started.elapsed()))
    };
    Ok(page(
        "Search every hive",
        None,
        &html! {
            h2 { "Search every hive" }
            p class="lede" {
                "One query, run in every hive's own file. Keep the comb open in another tab and "
                "watch the fleet open them a few at a time."
            }
            div class="panel" {
                form class="row" method="get" action="/search" {
                    input type="text" name="q" value=(query.q) placeholder="storm note 7"
                        autocomplete="off";
                    button class="btn" type="submit" { "Search" }
                }
            }
            @if let Some((by_hive, scanned, took)) = results {
                div class="panel" {
                    h3 {
                        (commas(by_hive.len() as u64)) " hives matched · "
                        (commas(scanned as u64)) " notes read · "
                        (took.as_millis()) " ms"
                    }
                    @if by_hive.is_empty() {
                        p class="empty" { "Nothing, anywhere." }
                    } @else {
                        ul class="hits" {
                            @for (hive, bodies) in by_hive.iter().take(200) {
                                li {
                                    a class="h" href=(format!("/enter/{hive}")) { (hive) }
                                    " — " (bodies.len()) " "
                                    (if bodies.len() == 1 { "match" } else { "matches" })
                                    @for b in bodies.iter().take(PER_HIVE) {
                                        div class="hint" { (b) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    ))
}
