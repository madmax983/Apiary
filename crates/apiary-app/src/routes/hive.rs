//! Your hive: the one the session says you are standing in.
//!
//! The page itself only needs the tenant and the fleet, so it renders even
//! for a hive whose file has been deleted (it says so, and where the backup
//! is). The notes are a separate fragment that goes through the generated
//! sharded repository, which is the part that needs the file to exist.
//!
//! Every route here is `#[public]` on purpose: Apiary has no login, and the
//! session's tenant is whichever hive you walked into. `autumn routes audit`
//! refuses an unclassified route, so the openness is declared rather than
//! accidental. Adding auth with a hive-membership check is the first item on
//! the README's "where this could go".

use autumn_web::AppState;
use autumn_web::download::Download;
use autumn_web::prelude::*;
use autumn_web::reexports::axum;
use autumn_web::tenancy::Tenant;
use hive::HiveName;
use serde::Deserialize;

use super::layout::{bytes, commas, page};
use crate::Apiary;
use crate::models::{NewNote, Note, NoteRepository as _, OffboardingRow, PgNoteRepository};
use crate::offboard;

/// The most notes the page shows.
pub const SHOWN: usize = 200;

fn hive_name(tenant: &Tenant) -> AutumnResult<HiveName> {
    HiveName::parse(&tenant.0).map_err(|e| AutumnError::bad_request_msg(e.to_string()))
}

/// The hive page.
#[get("/hive")]
#[public]
pub async fn index(
    State(state): State<AppState>,
    tenant: Tenant,
    mut db: Db,
    csrf: Option<CsrfToken>,
) -> AutumnResult<Markup> {
    let apiary = Apiary::of(&state)?;
    let name = hive_name(&tenant)?;
    let key = apiary
        .fleet
        .key_for(name.as_str())
        .map_err(|e| AutumnError::bad_request_msg(e.to_string()))?;
    let path = apiary.fleet.path_of(&key);
    let exists = apiary.fleet.exists(&key);
    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    let row = offboard::row_for(&mut db, name.as_str()).await?;

    Ok(page(
        name.as_str(),
        csrf.as_ref().map(CsrfToken::token),
        &html! {
            h2 { "The " code { (name) } " hive" }
            @if exists {
                p class="lede" {
                    "Everything below lives in one file of its own. The request that drew this "
                    "page asked the fleet for it; if it was closed, the fleet opened it, and it "
                    "will close it again when it needs the room."
                }
                div class="grid" {
                    div class="panel" {
                        h3 { "Notes" }
                        form hx-post="/hive/notes" hx-target="#notes" hx-swap="innerHTML"
                             data-reset {
                            div class="row" style="display:flex;gap:8px" {
                                input type="text" name="body" placeholder="write something into this hive"
                                    maxlength="500" required autocomplete="off";
                                button class="btn" type="submit" { "Write" }
                            }
                        }
                        div id="notes" hx-get="/hive/notes" hx-trigger="load" style="margin-top:12px" {
                            p class="hint" { "opening the file…" }
                        }
                    }
                    div {
                        div class="panel" {
                            h3 { "The file" }
                            dl class="kv" {
                                dt { "fleet name" } dd { code { (key.name()) } }
                                dt { "path" } dd { code { (path.display()) } }
                                dt { "size" } dd { (bytes(size)) }
                            }
                            p style="margin-top:12px" {
                                a class="btn quiet" href="/hive/export" { "Download this hive's file" }
                            }
                            p class="hint" {
                                "A consistent copy via the fleet's own backup, taken while the hive "
                                "stays live. It opens in any SQLite client."
                            }
                        }
                        (offboarding_panel(row.as_ref(), true))
                    }
                }
            } @else {
                p class="lede" { "This hive's file is not on disk." }
                (offboarding_panel(row.as_ref(), false))
                p { a href="/" { "Back to the comb" } }
            }
        },
    ))
}

/// The notes, through the generated tenant-scoped, sharded repository.
#[get("/hive/notes")]
#[public]
pub async fn notes(repo: PgNoteRepository) -> AutumnResult<Markup> {
    let mut all = repo.find_all().await?;
    Ok(notes_list(&mut all))
}

fn notes_list(all: &mut [Note]) -> Markup {
    all.sort_by_key(|n| std::cmp::Reverse(n.id));
    html! {
        @if all.is_empty() {
            p class="empty" { "An empty hive. Write the first note." }
        } @else {
            p class="hint" { (commas(all.len() as u64)) " notes" }
            ul class="notes" {
                @for n in all.iter().take(SHOWN) {
                    li {
                        span class="when" { (n.created_at.format("%m-%d %H:%M:%S")) }
                        span { (n.body) }
                    }
                }
            }
        }
    }
}

/// A new note's text.
#[derive(Debug, Deserialize)]
pub struct NoteForm {
    /// What to write.
    pub body: String,
}

/// Write a note into your hive.
#[post("/hive/notes")]
#[public]
pub async fn add_note(
    repo: PgNoteRepository,
    tenant: Tenant,
    Form(form): Form<NoteForm>,
) -> AutumnResult<Markup> {
    let body = form.body.trim();
    if body.is_empty() || body.len() > 500 {
        return Err(AutumnError::unprocessable_msg(
            "a note is 1 to 500 characters",
        ));
    }
    repo.save(&NewNote {
        body: body.to_owned(),
        tenant_id: Some(tenant.0.clone()),
        created_at: chrono::Utc::now().naive_utc(),
    })
    .await?;
    let mut all = repo.find_all().await?;
    Ok(notes_list(&mut all))
}

/// Download a consistent copy of your hive's file.
#[get("/hive/export")]
#[public]
pub async fn export(State(state): State<AppState>, tenant: Tenant) -> AutumnResult<Download> {
    let apiary = Apiary::of(&state)?;
    let name = hive_name(&tenant)?;
    let key = apiary
        .fleet
        .key_for(name.as_str())
        .map_err(|e| AutumnError::bad_request_msg(e.to_string()))?;
    if !apiary.fleet.exists(&key) {
        return Err(AutumnError::not_found_msg(format!(
            "hive {name} has no file"
        )));
    }
    let dir = tempfile_dir()?;
    let dest = dir.join(format!("{name}.db"));
    apiary
        .fleet
        .backup(&key, &dest)
        .await
        .map_err(|e| AutumnError::internal_server_error_msg(e.to_string()))?;
    let data = tokio::fs::read(&dest)
        .await
        .map_err(|e| AutumnError::internal_server_error_msg(e.to_string()))?;
    let _ = tokio::fs::remove_file(&dest).await;
    Ok(Download::from_bytes(data)
        .filename(format!("{name}.db"))
        .content_type("application/vnd.sqlite3"))
}

fn tempfile_dir() -> AutumnResult<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("apiary-exports");
    std::fs::create_dir_all(&dir)
        .map_err(|e| AutumnError::internal_server_error_msg(e.to_string()))?;
    Ok(dir)
}

/// Start offboarding your hive.
#[post("/hive/offboard")]
#[public]
pub async fn offboard(
    State(state): State<AppState>,
    tenant: Tenant,
    mut db: Db,
) -> AutumnResult<Markup> {
    let apiary = Apiary::of(&state)?;
    let name = hive_name(&tenant)?;
    let row = offboard::row_for(&mut db, name.as_str()).await?;
    if row.as_ref().is_some_and(|r| offboard::in_flight(&r.phase)) {
        return Ok(offboarding_panel(row.as_ref(), true));
    }
    apiary
        .driver
        .start(name.clone(), apiary.settings.grace)
        .await
        .map_err(|e| AutumnError::service_unavailable_msg(e.to_string()))?;
    Ok(html! {
        div class="panel" id="offboarding" hx-get="/_hive/offboarding" hx-trigger="every 1s"
            hx-swap="outerHTML" {
            h3 { "Offboarding" }
            p { span class="phase snapshotting" { "starting" } }
            p class="hint" { "Taking a backup first. Nothing is deleted until it checks out." }
        }
    })
}

/// Keep your hive: stop an offboarding during its grace period.
#[post("/hive/keep")]
#[public]
pub async fn keep(
    State(state): State<AppState>,
    tenant: Tenant,
    mut db: Db,
) -> AutumnResult<Markup> {
    let apiary = Apiary::of(&state)?;
    let name = hive_name(&tenant)?;
    let row = offboard::row_for(&mut db, name.as_str())
        .await?
        .ok_or_else(|| AutumnError::not_found_msg("this hive is not being offboarded"))?;
    apiary
        .driver
        .keep(row.workflow_id.clone())
        .await
        .map_err(|e| AutumnError::conflict_msg(e.to_string()))?;
    Ok(offboarding_panel(Some(&row), true))
}

/// Put a deleted hive back from its verified backup.
///
/// The backup is staged on the fleet's lifecycle hook and the hive is opened;
/// the fleet finds the file missing, asks the hook (under its path lock), and
/// then checks the restored file's identity row and migrates it exactly as it
/// would any file it opens.
#[post("/hive/restore")]
#[public]
pub async fn restore(
    State(state): State<AppState>,
    tenant: Tenant,
    mut db: Db,
) -> AutumnResult<axum::response::Response> {
    let apiary = Apiary::of(&state)?;
    let name = hive_name(&tenant)?;
    let key = apiary
        .fleet
        .key_for(name.as_str())
        .map_err(|e| AutumnError::bad_request_msg(e.to_string()))?;
    if apiary.fleet.exists(&key) {
        return Ok(
            html! { p class="hint" { "This hive's file is already here." } }.hx_redirect("/hive"),
        );
    }
    let row = offboard::row_for(&mut db, name.as_str())
        .await?
        .ok_or_else(|| AutumnError::not_found_msg("this hive was never offboarded"))?;
    let backup = row
        .backup
        .clone()
        .filter(|_| row.phase == "deleted")
        .ok_or_else(|| AutumnError::conflict_msg("there is no verified backup to restore"))?;
    apiary
        .watch
        .stage_restore(key.name(), std::path::PathBuf::from(&backup));
    let opened = apiary.fleet.open(&key).await;
    apiary.watch.unstage_restore(&key.name());
    opened.map_err(|e| AutumnError::internal_server_error_msg(e.to_string()))?;
    offboard::mark_restored(&mut db, &row, &backup).await?;
    Ok(html! { p class="ok" { "Restored." } }.hx_redirect("/hive"))
}

/// The offboarding panel on its own, for polling.
#[get("/_hive/offboarding")]
#[public]
pub async fn offboarding(tenant: Tenant, mut db: Db) -> AutumnResult<Markup> {
    let row = offboard::row_for(&mut db, &tenant.0).await?;
    Ok(offboarding_panel(row.as_ref(), true))
}

/// The offboarding panel: a button, a phase with a countdown, or what
/// happened.
pub fn offboarding_panel(row: Option<&OffboardingRow>, file_exists: bool) -> Markup {
    let polling = row.is_some_and(|r| offboard::in_flight(&r.phase));
    html! {
        div class="panel" id="offboarding"
            hx-get=[polling.then_some("/_hive/offboarding")]
            hx-trigger=[polling.then_some("every 1s")]
            hx-swap=[polling.then_some("outerHTML")] {
            h3 { "Offboarding" }
            @match row {
                None => {
                    p class="hint" {
                        "Offboarding backs the file up, checks the backup on its own, waits out a "
                        "grace period in which anybody can keep the hive, and only then deletes it. "
                        "It runs as a durable workflow, so a restart halfway through carries on."
                    }
                    @if file_exists { (offboard_button()) }
                }
                Some(r) => {
                    p { span class=(format!("phase {}", r.phase)) { (r.phase) } }
                    p class="hint" { (r.detail) }
                    @if r.phase == "grace" {
                        @if let Some(until) = r.grace_until {
                            p {
                                "Deleting in "
                                b data-until=(until.and_utc().timestamp_millis()) { "…" }
                                " unless somebody keeps it."
                            }
                        }
                        button class="btn" hx-post="/hive/keep" hx-target="#offboarding"
                            hx-swap="outerHTML" { "Keep this hive" }
                    }
                    @if !polling && file_exists && r.phase != "deleted" { (offboard_button()) }
                    @if !file_exists && r.phase == "deleted" && r.backup.is_some() {
                        button class="btn" hx-post="/hive/restore" hx-target="#offboarding"
                            hx-swap="beforeend" { "Restore from the backup" }
                    }
                }
            }
        }
    }
}

fn offboard_button() -> Markup {
    html! {
        button class="btn danger" hx-post="/hive/offboard" hx-target="#offboarding"
            hx-swap="outerHTML"
            hx-confirm="Back this hive up, wait out the grace period, then delete its file?" {
            "Offboard this hive"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(phase: &str) -> OffboardingRow {
        OffboardingRow {
            hive: "acme".into(),
            workflow_id: "offboard:acme:1".into(),
            phase: phase.into(),
            detail: "d".into(),
            backup: Some("apiary-archive/acme-1.db".into()),
            grace_until: chrono::DateTime::from_timestamp(1_700_000_000, 0).map(|d| d.naive_utc()),
            updated_at: chrono::Utc::now().naive_utc(),
        }
    }

    #[test]
    fn no_row_offers_the_button() {
        let html = offboarding_panel(None, true).into_string();
        assert!(html.contains("/hive/offboard"));
        assert!(!html.contains("every 1s"));
    }

    #[test]
    fn grace_counts_down_and_offers_keep_and_polls() {
        let html = offboarding_panel(Some(&row("grace")), true).into_string();
        assert!(html.contains("data-until=\"1700000000000\""));
        assert!(html.contains("/hive/keep"));
        assert!(html.contains("every 1s"));
        assert!(!html.contains("/hive/offboard\""));
    }

    #[test]
    fn a_kept_hive_can_be_offboarded_again_and_a_deleted_one_cannot() {
        assert!(
            offboarding_panel(Some(&row("kept")), true)
                .into_string()
                .contains("/hive/offboard")
        );
        assert!(
            !offboarding_panel(Some(&row("deleted")), false)
                .into_string()
                .contains("/hive/offboard")
        );
    }

    #[test]
    fn a_deleted_hive_with_a_backup_offers_a_restore() {
        let html = offboarding_panel(Some(&row("deleted")), false).into_string();
        assert!(html.contains("/hive/restore"));
        assert!(
            !offboarding_panel(Some(&row("deleted")), true)
                .into_string()
                .contains("/hive/restore")
        );
        let mut no_backup = row("deleted");
        no_backup.backup = None;
        assert!(
            !offboarding_panel(Some(&no_backup), false)
                .into_string()
                .contains("/hive/restore")
        );
    }

    #[test]
    fn notes_are_newest_first() {
        let t = chrono::Utc::now().naive_utc();
        let mut ns = vec![
            Note {
                id: 1,
                body: "first".into(),
                tenant_id: None,
                created_at: t,
            },
            Note {
                id: 2,
                body: "second".into(),
                tenant_id: None,
                created_at: t,
            },
        ];
        let html = notes_list(&mut ns).into_string();
        assert!(html.find("second").unwrap() < html.find("first").unwrap());
    }
}
