//! `HiveName` accepts a subset of what the fleet accepts.
//!
//! The Verus models cover the offboarding machine and the storm plan; this is
//! the edge they do not model — the fleet's own key rules — checked against
//! the real `DatabaseFleet` with generated names.

mod common;

use hive::HiveName;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    /// Anything `HiveName` accepts, the fleet accepts too, and names a file
    /// inside the fleet root.
    #[test]
    fn every_hive_name_is_a_fleet_key(raw in "[a-z][a-z0-9-]{0,44}") {
        let tmp = tempfile::tempdir().unwrap();
        let fleet = common::fleet(tmp.path(), 2);
        if let Ok(name) = HiveName::parse(&raw) {
            let key = fleet.key_for(name.as_str());
            prop_assert!(key.is_ok(), "{name}: {key:?}");
            let key = key.unwrap();
            prop_assert_eq!(key.name(), name.db_name());
            let root = tmp.path().join("fleet");
            prop_assert!(fleet.path_of(&key).starts_with(&root));
        }
    }

    /// The other direction, on arbitrary text: whatever the fleet refuses,
    /// `HiveName` refuses first.
    #[test]
    fn whatever_the_fleet_refuses_hive_name_refuses(raw in "\\PC{0,50}") {
        let tmp = tempfile::tempdir().unwrap();
        let fleet = common::fleet(tmp.path(), 2);
        if fleet.key_for(&raw).is_err() {
            prop_assert!(HiveName::parse(&raw).is_err(), "{raw:?}");
        }
    }
}
