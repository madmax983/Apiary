//! The comb: every hive on one page, lit by which files are open.

use std::convert::Infallible;

use autumn_web::AppState;
use autumn_web::db::fleet::FleetError;
use autumn_web::prelude::*;
use autumn_web::session::Session;
use autumn_web::sse::{Sse, from_subscriber};
use hive::{FleetMap, HiveName, StormPlan, Tile};
use serde::Deserialize;
use tokio_stream::Stream;

use super::TENANT_KEY;
use super::layout::{commas, page, stat};
use crate::Apiary;
use crate::storm::{self, StormReport};
use crate::watch::TOPIC;

/// Cells per row of the comb.
pub const COLUMNS: usize = 40;

/// The comb.
#[get("/")]
#[public]
pub async fn index(State(state): State<AppState>, csrf: Option<CsrfToken>) -> AutumnResult<Markup> {
    let apiary = Apiary::of(&state)?;
    Ok(page(
        "The comb",
        csrf.as_ref().map(CsrfToken::token),
        &html! {
            h2 { "The comb" }
            p class="lede" {
                "Each cell is one tenant's own SQLite file. A cell lights while the fleet has "
                "its file open and glows down when the fleet closes it — because it sat idle, or "
                "because the fleet needed the room for somebody else. The fleet tries to keep "
                b { (apiary.fleet.max_open()) } " open. Raise a storm and watch how far past that "
                "it runs, and how fast it comes back."
            }
            div hx-ext="sse" sse-connect="/stream" {
                div id="map" hx-get="/_map" hx-trigger="sse:message" hx-swap="innerHTML" {
                    (map_body(&apiary))
                }
            }
        },
    ))
}

/// The comb and its panels, for the live feed.
#[get("/_map")]
#[public]
pub async fn map_fragment(State(state): State<AppState>) -> AutumnResult<Markup> {
    let apiary = Apiary::of(&state)?;
    Ok(map_body(&apiary))
}

/// One message on every redraw-worthy change.
#[get("/stream")]
#[public]
pub async fn stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<autumn_web::sse::Event, Infallible>>> {
    from_subscriber(state.channels().subscribe(TOPIC))
}

/// The form behind "found a hive".
#[derive(Debug, Deserialize)]
pub struct FoundForm {
    /// The new hive's name.
    pub name: String,
}

/// Provision a hive and walk into it.
#[post("/hives")]
#[public]
pub async fn found(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<FoundForm>,
) -> AutumnResult<autumn_web::reexports::axum::response::Response> {
    let apiary = Apiary::of(&state)?;
    let name = match HiveName::parse(form.name.trim()) {
        Ok(n) => n,
        Err(e) => return Ok(html! { p class="err" { (e) } }.into_response()),
    };
    let key = apiary
        .fleet
        .key_for(name.as_str())
        .map_err(|e| AutumnError::bad_request_msg(e.to_string()))?;
    match apiary.fleet.provision(&key).await {
        Ok(_) | Err(FleetError::AlreadyExists { .. }) => {}
        Err(e) => return Ok(html! { p class="err" { (e) } }.into_response()),
    }
    session.insert(TENANT_KEY, name.as_str()).await;
    Ok(html! { p class="ok" { "Walking into " (name) "…" } }.hx_redirect("/hive"))
}

/// Walk into a hive (the cells link here). A deleted hive with a verified
/// backup can be walked into too — its page offers the restore — which is
/// what makes a ghost cell worth clicking.
#[get("/enter/{name}")]
#[public]
pub async fn enter(
    State(state): State<AppState>,
    session: Session,
    csrf: Option<CsrfToken>,
    mut db: Db,
    Path(raw): Path<String>,
) -> AutumnResult<autumn_web::reexports::axum::response::Response> {
    let apiary = Apiary::of(&state)?;
    let name = HiveName::parse(&raw).map_err(|e| AutumnError::bad_request_msg(e.to_string()))?;
    let key = apiary
        .fleet
        .key_for(name.as_str())
        .map_err(|e| AutumnError::bad_request_msg(e.to_string()))?;
    let exists = apiary.fleet.exists(&key);
    let restorable = !exists
        && crate::offboard::row_for(&mut db, name.as_str())
            .await?
            .is_some_and(|r| r.phase == "deleted" && r.backup.is_some());
    if !exists && !restorable {
        return Ok((
            StatusCode::NOT_FOUND,
            page(
                "No such hive",
                csrf.as_ref().map(CsrfToken::token),
                &html! {
                    h2 { "There is no hive called " code { (name) } }
                    p class="lede" {
                        "The fleet runs with " code { "create_on_demand = false" } ": a hive's "
                        "file exists only once somebody founds it, and walking into one that does "
                        "not exist is a 404 rather than a new empty file."
                    }
                    (found_form(Some(name.as_str())))
                },
            ),
        )
            .into_response());
    }
    session.insert(TENANT_KEY, name.as_str()).await;
    Ok(Redirect::to("/hive").into_response())
}

/// Raise a storm.
#[post("/storm")]
#[public]
pub async fn raise_storm(State(state): State<AppState>) -> AutumnResult<Markup> {
    let apiary = Apiary::of(&state)?;
    let seed = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(1);
    let started = storm::start(
        apiary.fleet.clone(),
        std::sync::Arc::clone(&apiary.storm),
        StormPlan::default_storm(seed),
    );
    if !started {
        tracing::info!("a storm is already running");
    }
    Ok(storm_panel(&apiary.storm.report()))
}

fn found_form(prefill: Option<&str>) -> Markup {
    html! {
        div class="panel" {
            h3 { "Found a hive" }
            form class="row" hx-post="/hives" hx-target="#found-msg" hx-swap="innerHTML" {
                input type="text" name="name" placeholder="acme" value=(prefill.unwrap_or(""))
                    autocomplete="off" spellcheck="false" maxlength="40" required;
                button class="btn" type="submit" { "Found" }
            }
            div id="found-msg" {}
            p class="hint" { "a–z, 0–9 and single hyphens; starts with a letter. One file per hive." }
        }
    }
}

fn map_body(apiary: &Apiary) -> Markup {
    let map = apiary.watch.snapshot();
    let now = apiary.watch.now_ms();
    let tiles = map.tiles(now);
    let stats = apiary.fleet.stats();
    let report = apiary.storm.report();
    html! {
        div class="grid" {
            div class="panel" {
                h3 { (commas(tiles.len() as u64)) " hives on disk" }
                (comb(&tiles))
                div class="legend" {
                    span { span class="t warm" { i {} } " open" }
                    span { span class="t cooling" { i {} } " just closed" }
                    span { span class="t cold" { i {} } " at rest" }
                    span { span class="t gone" { i {} } " deleted" }
                }
            }
            div {
                div class="panel" {
                    h3 { "Open files" }
                    div class="count" { (stats.open) " / " (stats.max_open) }
                    div class="gauge" { i style=(gauge_style(stats.open, stats.max_open)) {} }
                    p class="hint" {
                        "The fleet's own count. max_open is a soft ceiling: a file used in the last "
                        "two seconds is never closed to make room, so a burst runs past it, and the "
                        "sweeper brings it back down once the burst moves on."
                    }
                    (stats_tiles(&map, &stats))
                }
                (storm_panel(&report))
                (found_form(None))
            }
        }
    }
}

fn gauge_style(open: usize, max: usize) -> String {
    let pct = (open * 100).checked_div(max).unwrap_or(0).min(100);
    format!("width:{pct}%")
}

fn stats_tiles(map: &FleetMap, stats: &autumn_web::db::fleet::FleetStats) -> Markup {
    html! {
        div class="stats" style="margin-top:12px" {
            (stat(map.peak_open(), "peak open", true))
            (stat(commas(stats.opens_total), "opens", false))
            (stat(commas(stats.closes_total), "closes", false))
            (stat(commas(stats.created_total), "founded", false))
            (stat(commas(stats.deleted_total), "deleted", false))
            (stat(commas(stats.open_failures_total), "failed opens", false))
        }
    }
}

/// The storm panel.
pub fn storm_panel(r: &StormReport) -> Markup {
    let pct = (r.done * 100).checked_div(r.total).unwrap_or(0).min(100);
    html! {
        div class="panel" id="storm" {
            h3 { "The storm" }
            p class="hint" {
                "1,000 hives, 6,000 touches, 70% of them on 24 hot hives, eight at a time: a "
                "working set far bigger than the fleet will hold. Every hive is founded before it "
                "is touched. Watch how far past the cap the open count runs, and how fast it "
                "comes back."
            }
            @if r.runs > 0 {
                div class="bar" { i style=(format!("width:{pct}%")) {} }
                div class="stats" {
                    (stat(format!("{} / {}", commas(r.done), commas(r.total)), "ops", r.running))
                    (stat(format!("{}/s", commas(r.rate())), "rate", false))
                    (stat(format!("{} / {}", r.peak_open, r.max_open), "peak open / cap", false))
                    (stat(r.over_cap, "over the cap", r.over_cap > 0))
                    (stat(commas(r.written), "notes written", false))
                    (stat(commas(r.failed), "failed ops", r.failed > 0))
                }
                @if let Some(e) = &r.last_error {
                    p class="err" { (e) }
                }
            }
            div style="margin-top:12px" {
                @if r.running {
                    button class="btn" disabled { "Storming…" }
                } @else {
                    button class="btn" hx-post="/storm" hx-target="#storm" hx-swap="outerHTML" {
                        @if r.runs == 0 { "Raise a storm" } @else { "Raise another" }
                    }
                }
            }
        }
    }
}

/// The comb itself: rows of [`COLUMNS`] cells, every other row offset half a
/// cell.
pub fn comb(tiles: &[Tile]) -> Markup {
    html! {
        @if tiles.is_empty() {
            p class="empty" { "No hives yet. Found one, or raise a storm." }
        } @else {
            div class="comb" {
                @for (row, chunk) in tiles.chunks(COLUMNS).enumerate() {
                    div class=(if row % 2 == 1 { "r o" } else { "r" }) {
                        @for t in chunk {
                            a class=(format!("t {}", t.state.class()))
                              href=(format!("/enter/{}", t.name))
                              title=(format!("{} — opened {}×", t.name, t.opens)) { i {} }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive::{MapEvent, TileState};

    #[test]
    fn the_comb_offsets_every_other_row() {
        let mut m = FleetMap::new();
        let cells = u32::try_from(COLUMNS * 2 + 3).unwrap();
        for n in 1..=cells {
            m.apply(MapEvent::Known(HiveName::storm(n)), 0);
        }
        let html = comb(&m.tiles(0)).into_string();
        assert_eq!(html.matches(r#"class="r""#).count(), 2);
        assert_eq!(html.matches(r#"class="r o""#).count(), 1);
        assert_eq!(html.matches("/enter/storm-").count(), COLUMNS * 2 + 3);
    }

    #[test]
    fn cells_carry_their_state() {
        let mut m = FleetMap::new();
        m.apply(MapEvent::Opened(HiveName::parse("acme").unwrap()), 1);
        let tiles = m.tiles(2);
        assert_eq!(tiles[0].state, TileState::Warm);
        assert!(comb(&tiles).into_string().contains(r#"class="t warm""#));
    }

    #[test]
    fn the_gauge_is_clamped() {
        assert_eq!(gauge_style(0, 0), "width:0%");
        assert_eq!(gauge_style(8, 16), "width:50%");
        assert_eq!(gauge_style(20, 16), "width:100%");
    }

    #[test]
    fn the_storm_button_is_disabled_while_running() {
        let running = StormReport {
            running: true,
            runs: 1,
            total: 10,
            done: 5,
            ..StormReport::default()
        };
        let html = storm_panel(&running).into_string();
        assert!(html.contains("disabled"));
        assert!(html.contains("width:50%"));
        let idle = storm_panel(&StormReport::default()).into_string();
        assert!(idle.contains("Raise a storm"));
    }
}
