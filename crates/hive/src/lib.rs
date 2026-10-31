//! Apiary's engine: the pure rules a fleet of per-tenant `SQLite` files is run by.
//!
//! Nothing here touches a file, a socket, a clock or an async runtime. The app
//! crate does all of that and asks this crate what is allowed:
//!
//! - [`name`] — which strings may name a hive, so a hive name always names a
//!   fleet file the framework will accept.
//! - [`offboard`] — the offboarding state machine. The one irreversible thing
//!   Apiary does is delete a tenant's database file; this module is the only
//!   place that may say yes, and `verus/src/offboard.rs` proves when it does.
//! - [`storm`] — the deterministic workload behind the storm button. It
//!   always provisions a hive before it touches it, which
//!   `verus/src/storm.rs` proves.
//! - [`map`] — the live picture of the fleet: which hives are open, which just
//!   closed, which are gone.
#![forbid(unsafe_code)]

pub mod map;
pub mod name;
pub mod offboard;
pub mod storm;

pub use map::{FleetMap, MapEvent, Tile, TileState};
pub use name::{HiveName, HiveNameError};
pub use offboard::{Offboarding, Phase, Step, StepError};
pub use storm::{Op, StormPlan, StormPlanError};
