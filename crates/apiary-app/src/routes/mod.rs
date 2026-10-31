//! HTTP surface.
//!
//! Nothing here has a login: the session holds which hive you are standing
//! in, and anybody can walk into any hive. That is the demo, and it is the
//! first thing on the README's list of where this could go.

pub mod about;
pub mod assets;
pub mod comb;
pub mod hive;
pub mod layout;
pub mod search;

/// Every handler `main.rs` must register (checked by a test in `lib.rs`).
pub const ALL: &[&str] = &[
    "routes::comb::index",
    "routes::comb::map_fragment",
    "routes::comb::stream",
    "routes::comb::found",
    "routes::comb::enter",
    "routes::comb::raise_storm",
    "routes::hive::index",
    "routes::hive::notes",
    "routes::hive::add_note",
    "routes::hive::export",
    "routes::hive::offboard",
    "routes::hive::keep",
    "routes::hive::restore",
    "routes::hive::offboarding",
    "routes::search::index",
    "routes::about::show",
    "routes::assets::script",
];

/// The session key the tenancy middleware reads the hive from. Matches
/// `[tenancy] session_key` in `autumn.toml`.
pub const TENANT_KEY: &str = "tenant_id";
