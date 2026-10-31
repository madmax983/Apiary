//! The offboarding workflow end to end, on the embedded Harvest runtime and a
//! real fleet: delete after grace, keep during grace, and restore from the
//! verified backup.
//!
//! One test function on purpose: the activities' dependencies are installed
//! once per process, so every scenario shares one fleet and runs in order.

mod common;

use std::time::{Duration, Instant};

use apiary_app::models::OffboardingRow;
use apiary_app::offboard;
use autumn_web::db::fleet::DatabaseFleet;
use autumn_web::test::TestClient;

async fn found_with_notes(client: &TestClient, hive: &str, notes: &[&str]) {
    client
        .post("/hives")
        .header("hx-request", "true")
        .form(&format!("name={hive}"))
        .send()
        .await
        .assert_status(200);
    for n in notes {
        client
            .post("/hive/notes")
            .header("x-tenant-id", hive)
            .form(&format!("body={n}"))
            .send()
            .await
            .assert_status(200);
    }
}

async fn row(pool: &Pool, hive: &str) -> Option<OffboardingRow> {
    let mut conn = pool.get().await.unwrap();
    offboard::row_for(&mut conn, hive).await.unwrap()
}

async fn wait_for_phase(pool: &Pool, hive: &str, phase: &str, within: Duration) -> OffboardingRow {
    let started = Instant::now();
    loop {
        if let Some(r) = row(pool, hive).await
            && r.phase == phase
        {
            return r;
        }
        assert!(
            started.elapsed() < within,
            "{hive} never reached {phase}; last row: {:?}",
            row(pool, hive).await
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

type Pool = autumn_web::reexports::diesel_async::pooled_connection::deadpool::Pool<
    autumn_web::db::RuntimeConnection,
>;

/// The grace every scenario runs with: long enough that a keep posted as soon
/// as `grace` is observed lands well inside it on a slow debug build.
const GRACE_SECS: u64 = 4;
const PATIENCE: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offboarding_deletes_only_after_a_verified_backup_and_the_grace() {
    let tmp = tempfile::tempdir().unwrap();
    let fleet = common::fleet(tmp.path(), 4);
    let control = common::control_pool(tmp.path()).await;
    let client = common::app(
        control.clone(),
        fleet.clone(),
        common::settings(tmp.path(), GRACE_SECS),
    );

    delete_after_grace_then_restore(&client, &fleet, &control).await;
    keep_wins_during_grace(&client, &fleet, &control).await;
    a_second_offboard_while_running_starts_nothing(&client, &fleet, &control).await;
}

async fn post(client: &TestClient, path: &str, hive: &str) -> autumn_web::test::TestResponse {
    let r = client.post(path).header("x-tenant-id", hive).send().await;
    r.assert_status(200);
    r
}

/// Backup, check, grace, delete — then the backup goes back in through the
/// fleet's own restore hook.
async fn delete_after_grace_then_restore(
    client: &TestClient,
    fleet: &DatabaseFleet,
    control: &Pool,
) {
    found_with_notes(client, "acme", &["one", "two", "three"]).await;
    let key = fleet.key_for("acme").unwrap();
    post(client, "/hive/offboard", "acme").await;

    let grace = wait_for_phase(control, "acme", "grace", PATIENCE).await;
    assert!(fleet.exists(&key), "the file is still there during grace");
    let backup = grace.backup.clone().expect("a verified backup is recorded");
    assert!(std::path::Path::new(&backup).is_file());
    offboard::check_backup(&backup, 3).expect("the recorded backup passes the check");
    assert!(grace.grace_until.is_some());

    let deleted = wait_for_phase(control, "acme", "deleted", PATIENCE).await;
    assert!(!fleet.exists(&key), "the file is gone after grace");
    assert_eq!(deleted.backup.as_deref(), Some(backup.as_str()));
    client
        .get("/hive/notes")
        .header("x-tenant-id", "acme")
        .send()
        .await
        .assert_status(404);
    client
        .get("/hive")
        .header("x-tenant-id", "acme")
        .send()
        .await
        .assert_status(200)
        .assert_body_contains("/hive/restore");

    post(client, "/hive/restore", "acme").await;
    assert!(fleet.exists(&key));
    client
        .get("/hive/notes")
        .header("x-tenant-id", "acme")
        .send()
        .await
        .assert_status(200)
        .assert_body_contains("one")
        .assert_body_contains("three");
    assert_eq!(row(control, "acme").await.unwrap().phase, "restored");
}

/// A keep during grace wins: the run ends `kept` and the file stays, past the
/// moment the grace would have ended.
async fn keep_wins_during_grace(client: &TestClient, fleet: &DatabaseFleet, control: &Pool) {
    found_with_notes(client, "globex", &["keep me"]).await;
    let key = fleet.key_for("globex").unwrap();
    post(client, "/hive/offboard", "globex").await;
    wait_for_phase(control, "globex", "grace", PATIENCE).await;
    post(client, "/hive/keep", "globex").await;
    let kept = wait_for_phase(control, "globex", "kept", PATIENCE).await;
    tokio::time::sleep(Duration::from_secs(GRACE_SECS + 1)).await;
    assert!(fleet.exists(&key), "a kept hive keeps its file");
    assert_eq!(row(control, "globex").await.unwrap().phase, "kept");
    assert!(
        kept.backup.is_some(),
        "the backup taken before the keep is still recorded"
    );
    client
        .get("/hive/notes")
        .header("x-tenant-id", "globex")
        .send()
        .await
        .assert_status(200)
        .assert_body_contains("keep me");
}

/// Offboarding a hive whose offboarding is already running shows its panel
/// and starts no second workflow.
async fn a_second_offboard_while_running_starts_nothing(
    client: &TestClient,
    fleet: &DatabaseFleet,
    control: &Pool,
) {
    found_with_notes(client, "initech", &[]).await;
    post(client, "/hive/offboard", "initech").await;
    let first = wait_for_phase(control, "initech", "grace", PATIENCE).await;
    post(client, "/hive/offboard", "initech")
        .await
        .assert_body_contains("grace");
    let done = wait_for_phase(control, "initech", "deleted", PATIENCE).await;
    assert_eq!(done.workflow_id, first.workflow_id, "one workflow, not two");
    assert!(!fleet.exists(&fleet.key_for("initech").unwrap()));
}
