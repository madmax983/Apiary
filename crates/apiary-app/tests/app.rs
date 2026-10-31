//! Apiary over a real fleet, through HTTP.
//!
//! Every hive here is a real `SQLite` file in a temporary directory, opened,
//! migrated and closed by autumn-web's `DatabaseFleet`.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use apiary_app::models::Count;
use apiary_app::storm::{self, StormState};
use autumn_web::reexports::{diesel, diesel_async};
use diesel_async::RunQueryDsl as _;
use hive::{HiveName, Op, StormPlan};

async fn found(
    client: &autumn_web::test::TestClient,
    name: &str,
) -> autumn_web::test::TestResponse {
    let body = format!("name={name}");
    client
        .post("/hives")
        .header("hx-request", "true")
        .form(&body)
        .send()
        .await
}

#[tokio::test]
async fn founding_a_hive_creates_exactly_its_file() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 4);
    let client = common::app(
        common::control_pool(tmp.path()).await,
        fleet.clone(),
        common::settings(tmp.path(), 1),
    );

    let r = found(&client, "acme").await;
    r.assert_status(200);
    assert_eq!(r.header("hx-redirect"), Some("/hive"));
    found(&client, "acme").await.assert_status(200); // a second founding just walks in

    let names: Vec<String> = fleet
        .list()
        .await
        .unwrap()
        .iter()
        .map(autumn_web::fleet_layout::FleetDbKey::name)
        .collect();
    assert_eq!(names, vec!["tenant:acme"]);
    assert!(fleet.path_of(&fleet.key_for("acme").unwrap()).is_file());
}

#[tokio::test]
async fn names_the_fleet_would_refuse_never_reach_it() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 4);
    let client = common::app(
        common::control_pool(tmp.path()).await,
        fleet.clone(),
        common::settings(tmp.path(), 1),
    );
    for bad in ["Acme", "..", "a.b", "con", "a--b", "x-"] {
        let r = found(&client, bad).await;
        r.assert_status(200).assert_body_contains("class=\"err\"");
    }
    assert!(
        fleet.list().await.unwrap().is_empty(),
        "nothing created for a refused name"
    );
}

#[tokio::test]
async fn notes_go_through_the_sharded_repository_into_each_hives_own_file() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 4);
    let client = common::app(
        common::control_pool(tmp.path()).await,
        fleet.clone(),
        common::settings(tmp.path(), 1),
    );
    found(&client, "acme").await.assert_status(200);
    found(&client, "globex").await.assert_status(200);

    for (hive, body) in [("acme", "plan"), ("globex", "launch"), ("acme", "ship")] {
        client
            .post("/hive/notes")
            .header("x-tenant-id", hive)
            .form(&format!("body={body}"))
            .send()
            .await
            .assert_status(200);
    }
    let acme = client
        .get("/hive/notes")
        .header("x-tenant-id", "acme")
        .send()
        .await;
    acme.assert_status(200)
        .assert_body_contains("plan")
        .assert_body_contains("ship");
    assert!(
        !acme.text().contains("launch"),
        "a globex note leaked into acme"
    );
    let globex = client
        .get("/hive/notes")
        .header("x-tenant-id", "globex")
        .send()
        .await
        .text();
    assert!(globex.contains("launch") && !globex.contains("plan"));

    // And on disk: two rows in acme's file, one in globex's, none in control.
    assert_eq!(count(&fleet, "acme").await, 2);
    assert_eq!(count(&fleet, "globex").await, 1);
}

#[tokio::test]
async fn an_unfounded_hive_is_a_404_and_a_hostile_one_a_400() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 4);
    let client = common::app(
        common::control_pool(tmp.path()).await,
        fleet.clone(),
        common::settings(tmp.path(), 1),
    );
    client
        .get("/hive/notes")
        .header("x-tenant-id", "ghost")
        .send()
        .await
        .assert_status(404);
    client
        .get("/hive/notes")
        .header("x-tenant-id", "..")
        .send()
        .await
        .assert_status(400);
    let r = client.get("/enter/ghost").send().await;
    assert_eq!(
        r.status.as_u16(),
        404,
        "{}",
        r.text().chars().take(600).collect::<String>()
    );
    client.get("/enter/Ghost").send().await.assert_status(400);
    assert!(fleet.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_comb_draws_one_cell_per_hive_and_the_fleet_health() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 4);
    let client = common::app(
        common::control_pool(tmp.path()).await,
        fleet,
        common::settings(tmp.path(), 1),
    );
    for n in ["a1", "a2", "a3"] {
        found(&client, n).await.assert_status(200);
    }
    let map = client.get("/_map").send().await;
    map.assert_status(200);
    assert_eq!(map.text().matches("href=\"/enter/").count(), 3);
    let health: serde_json::Value = client.get("/actuator/health").send().await.json();
    assert_eq!(
        health["components"]["db:fleet"]["details"]["created_total"], 3,
        "{health}"
    );
}

#[tokio::test]
async fn export_is_a_real_sqlite_file_holding_the_hives_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 4);
    let client = common::app(
        common::control_pool(tmp.path()).await,
        fleet,
        common::settings(tmp.path(), 1),
    );
    found(&client, "acme").await.assert_status(200);
    client
        .post("/hive/notes")
        .header("x-tenant-id", "acme")
        .form("body=exported")
        .send()
        .await
        .assert_status(200);
    let r = client
        .get("/hive/export")
        .header("x-tenant-id", "acme")
        .send()
        .await;
    r.assert_status(200);
    assert!(
        r.header("content-disposition")
            .unwrap_or("")
            .contains("acme.db")
    );
    let path = tmp.path().join("exported.db");
    std::fs::write(&path, &r.body).unwrap();
    assert!(
        std::fs::read(&path)
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );
    apiary_app::offboard::check_backup(path.to_str().unwrap(), 1)
        .expect("the export passes the backup check");
}

#[tokio::test]
async fn search_runs_the_repository_across_every_hive() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 2);
    let client = common::app(
        common::control_pool(tmp.path()).await,
        fleet,
        common::settings(tmp.path(), 1),
    );
    for (hive, body) in [
        ("h1", "honey"),
        ("h2", "wax"),
        ("h3", "honeycomb"),
        ("h4", "drone"),
    ] {
        found(&client, hive).await.assert_status(200);
        client
            .post("/hive/notes")
            .header("x-tenant-id", hive)
            .form(&format!("body={body}"))
            .send()
            .await
            .assert_status(200);
    }
    let r = client.get("/search?q=honey").send().await;
    r.assert_status(200).assert_body_contains("2 hives matched");
    let html = r.text();
    assert!(
        html.contains("/enter/h1") && html.contains("/enter/h3") && !html.contains("/enter/h2")
    );
}

/// A storm through a fleet that keeps three files open: every op lands,
/// every hive holds exactly the notes the plan wrote to it and only those,
/// and once the fleet settles it is back under its cap.
#[tokio::test]
async fn a_small_storm_lands_every_op_in_the_right_file() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 3);
    // Small enough for a default macOS `ulimit -n` of 256.
    let plan = StormPlan::new(40, 400, 6, 70, 60, 42).unwrap();
    let state = Arc::new(StormState::default());
    storm::run(&fleet, &state, plan).await;
    let report = state.report();
    assert_eq!(report.failed, 0, "{:?}", report.last_error);
    assert_eq!(report.done, report.total);

    let mut expected: BTreeMap<u32, i64> = BTreeMap::new();
    for op in plan.ops() {
        match op {
            Op::Provision(h) => {
                expected.entry(h).or_insert(0);
            }
            Op::Write { hive, .. } => *expected.entry(hive).or_insert(0) += 1,
            Op::Read(_) => {}
        }
    }
    assert_eq!(fleet.list().await.unwrap().len(), expected.len());
    for (h, n) in &expected {
        let name = HiveName::storm(h + 1);
        assert_eq!(count(&fleet, name.as_str()).await, *n, "{name}");
        assert_eq!(
            foreign_rows(&fleet, name.as_str()).await,
            0,
            "{name} holds another hive's note"
        );
    }

    let _ = fleet.close_idle();
    assert!(
        fleet.stats().open <= fleet.max_open(),
        "{:?}",
        fleet.stats()
    );
}

/// Control-plane rows never land in a hive, and notes never land in control
/// (the one migration set puts both tables in both places; see ADR 0002).
#[tokio::test]
async fn control_and_hive_tables_stay_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 4);
    let control = common::control_pool(tmp.path()).await;
    let client = common::app(
        control.clone(),
        fleet.clone(),
        common::settings(tmp.path(), 1),
    );
    found(&client, "acme").await.assert_status(200);
    client
        .post("/hive/notes")
        .header("x-tenant-id", "acme")
        .form("body=x")
        .send()
        .await
        .assert_status(200);
    let mut conn = control.get().await.unwrap();
    let in_control: Count = diesel::sql_query("SELECT COUNT(*) AS n FROM notes")
        .get_result(&mut *conn)
        .await
        .unwrap();
    assert_eq!(in_control.n, 0);
    let db = fleet.open(&fleet.key_for("acme").unwrap()).await.unwrap();
    let mut hconn = db.pool().get().await.unwrap();
    let in_hive: Count = diesel::sql_query("SELECT COUNT(*) AS n FROM offboardings")
        .get_result(&mut *hconn)
        .await
        .unwrap();
    assert_eq!(in_hive.n, 0);
}

async fn count(fleet: &autumn_web::db::fleet::DatabaseFleet, hive: &str) -> i64 {
    let db = fleet.open(&fleet.key_for(hive).unwrap()).await.unwrap();
    let mut conn = db.pool().get().await.unwrap();
    let c: Count = diesel::sql_query("SELECT COUNT(*) AS n FROM notes")
        .get_result(&mut *conn)
        .await
        .unwrap();
    c.n
}

async fn foreign_rows(fleet: &autumn_web::db::fleet::DatabaseFleet, hive: &str) -> i64 {
    let db = fleet.open(&fleet.key_for(hive).unwrap()).await.unwrap();
    let mut conn = db.pool().get().await.unwrap();
    let c: Count = diesel::sql_query("SELECT COUNT(*) AS n FROM notes WHERE tenant_id IS NOT ?")
        .bind::<diesel::sql_types::Text, _>(hive)
        .get_result(&mut *conn)
        .await
        .unwrap();
    c.n
}
