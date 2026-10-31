//! The two kinds of row Apiary keeps.

use chrono::NaiveDateTime;
use diesel::{Insertable, Queryable, QueryableByName, Selectable};
use serde::Serialize;

use crate::schema::{notes, offboardings};

/// A note in a hive. Lives in that hive's own database file.
#[autumn_web::model]
pub struct Note {
    /// Row id, unique within the hive's file only.
    #[id]
    pub id: i64,
    /// What was written.
    pub body: String,
    /// The hive, for `#[repository(tenant_scoped)]`.
    pub tenant_id: Option<String>,
    /// When it was written (UTC).
    pub created_at: NaiveDateTime,
}

/// The generated repository is tenant-scoped (every query filters on the
/// request's tenant) and sharded (it runs on the request's hive file, which
/// the fleet opens on first use). `CrossShard<PgNoteRepository>` fans the
/// same queries out over every hive on disk.
#[autumn_web::repository(Note, tenant_scoped, sharded)]
pub trait NoteRepository {}

/// Where a hive's offboarding is. Lives in the control database.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable, Insertable, Serialize)]
#[diesel(table_name = offboardings)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct OffboardingRow {
    /// The hive being offboarded.
    pub hive: String,
    /// The Harvest workflow id driving it.
    pub workflow_id: String,
    /// [`hive::Phase::label`] of the current phase.
    pub phase: String,
    /// One line about the phase: the backup's path, why it aborted, …
    pub detail: String,
    /// The verified backup's path, once there is one.
    pub backup: Option<String>,
    /// When the grace period ends, once it has started (UTC).
    pub grace_until: Option<NaiveDateTime>,
    /// When the row last changed (UTC).
    pub updated_at: NaiveDateTime,
}

/// A note as the storm and the search see it, outside any one request's hive.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName, Serialize)]
pub struct NoteRow {
    /// Row id within its hive.
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub id: i64,
    /// What was written.
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub body: String,
}

/// `SELECT COUNT(*) AS n`.
#[derive(Debug, Clone, Copy, QueryableByName)]
pub struct Count {
    /// The count.
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub n: i64,
}
