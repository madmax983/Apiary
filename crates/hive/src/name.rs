//! Hive names.
//!
//! A hive is one tenant, and one tenant is one `SQLite` file under the fleet
//! root, so a hive name ends up in a path. `autumn-web` refuses a routing key
//! that cannot name a file (`..`, upper case, dots, Windows device names) with
//! a `400`, and creates nothing. Apiary is stricter than that on purpose:
//! [`HiveName`] accepts a subset of what the fleet accepts, so a name that got
//! past this type can never be the reason a fleet operation fails.
//! `tests/name_vs_fleet.rs` in the app crate checks the subset claim against
//! the real `DatabaseFleet` with generated names.

use std::fmt;

/// The longest hive name.
pub const MAX_LEN: usize = 40;

/// Names Windows reserves for devices. A file called `con.db` cannot be
/// created there, and the fleet refuses them on every platform so that a
/// fleet directory can be copied between machines.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A validated hive (tenant) name: `[a-z][a-z0-9-]*`, at most
/// [`MAX_LEN`] bytes, no `--`, no trailing `-`, not a reserved device name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HiveName(String);

/// Why a string is not a hive name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HiveNameError {
    /// The name is empty.
    #[error("a hive needs a name")]
    Empty,
    /// The name is longer than [`MAX_LEN`].
    #[error("a hive name is at most {MAX_LEN} characters")]
    TooLong,
    /// The name does not start with a lower-case letter.
    #[error("a hive name starts with a lower-case letter")]
    BadStart,
    /// A character outside `a-z`, `0-9` and `-`.
    #[error("a hive name uses only a-z, 0-9 and '-' (found {0:?})")]
    BadChar(char),
    /// `--`, or a `-` at the end.
    #[error("a hive name has no '--' and does not end in '-'")]
    BadHyphen,
    /// A Windows device name such as `con`.
    #[error("{0:?} is reserved by Windows and cannot name a file")]
    Reserved(String),
}

impl HiveName {
    /// Check `raw` and wrap it.
    ///
    /// # Errors
    ///
    /// The first rule `raw` breaks, in the order the variants of
    /// [`HiveNameError`] are listed.
    pub fn parse(raw: &str) -> Result<Self, HiveNameError> {
        if raw.is_empty() {
            return Err(HiveNameError::Empty);
        }
        if raw.len() > MAX_LEN {
            return Err(HiveNameError::TooLong);
        }
        let mut chars = raw.chars();
        match chars.next() {
            Some(c) if c.is_ascii_lowercase() => {}
            _ => return Err(HiveNameError::BadStart),
        }
        if let Some(bad) = raw
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
        {
            return Err(HiveNameError::BadChar(bad));
        }
        if raw.contains("--") || raw.ends_with('-') {
            return Err(HiveNameError::BadHyphen);
        }
        if RESERVED.contains(&raw) {
            return Err(HiveNameError::Reserved(raw.to_owned()));
        }
        Ok(Self(raw.to_owned()))
    }

    /// The name, as the fleet's routing key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name the fleet gives this hive's database: `tenant:<name>`.
    #[must_use]
    pub fn db_name(&self) -> String {
        format!("tenant:{}", self.0)
    }

    /// The hive a fleet database name belongs to, if it is a tenant database
    /// whose name is a valid hive name.
    #[must_use]
    pub fn from_db_name(db_name: &str) -> Option<Self> {
        db_name
            .strip_prefix("tenant:")
            .and_then(|n| Self::parse(n).ok())
    }

    /// The `n`th hive the storm uses: `storm-0001` and on.
    #[must_use]
    pub fn storm(n: u32) -> Self {
        Self(format!("storm-{n:04}"))
    }
}

impl fmt::Display for HiveName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for HiveName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn ordinary_names_parse() {
        for ok in ["acme", "globex", "initech-2", "a", "storm-0001", "z9"] {
            assert_eq!(HiveName::parse(ok).map(|n| n.0), Ok(ok.to_owned()), "{ok}");
        }
    }

    #[test]
    fn each_rule_has_its_own_refusal() {
        assert_eq!(HiveName::parse(""), Err(HiveNameError::Empty));
        assert_eq!(
            HiveName::parse(&"a".repeat(MAX_LEN + 1)),
            Err(HiveNameError::TooLong)
        );
        assert_eq!(HiveName::parse("Acme"), Err(HiveNameError::BadStart));
        assert_eq!(HiveName::parse("9lives"), Err(HiveNameError::BadStart));
        assert_eq!(HiveName::parse("-a"), Err(HiveNameError::BadStart));
        assert_eq!(HiveName::parse(".."), Err(HiveNameError::BadStart));
        assert_eq!(HiveName::parse("a.b"), Err(HiveNameError::BadChar('.')));
        assert_eq!(HiveName::parse("a/b"), Err(HiveNameError::BadChar('/')));
        assert_eq!(HiveName::parse("a_b"), Err(HiveNameError::BadChar('_')));
        assert_eq!(HiveName::parse("a--b"), Err(HiveNameError::BadHyphen));
        assert_eq!(HiveName::parse("ab-"), Err(HiveNameError::BadHyphen));
        assert_eq!(
            HiveName::parse("con"),
            Err(HiveNameError::Reserved("con".into()))
        );
        assert_eq!(
            HiveName::parse("lpt9"),
            Err(HiveNameError::Reserved("lpt9".into()))
        );
    }

    #[test]
    fn the_longest_name_is_allowed() {
        assert!(HiveName::parse(&"a".repeat(MAX_LEN)).is_ok());
    }

    #[test]
    fn db_names_round_trip() {
        let n = HiveName::parse("acme").unwrap();
        assert_eq!(n.db_name(), "tenant:acme");
        assert_eq!(HiveName::from_db_name("tenant:acme"), Some(n));
        assert_eq!(HiveName::from_db_name("slot:00042"), None);
        assert_eq!(HiveName::from_db_name("tenant:Acme"), None);
    }

    #[test]
    fn storm_names_are_hive_names() {
        for n in [0, 1, 999, 1000, 9999] {
            let s = HiveName::storm(n);
            assert_eq!(HiveName::parse(s.as_str()), Ok(s));
        }
    }

    proptest! {
        /// Whatever is accepted is exactly what was given, and re-parses.
        #[test]
        fn accepted_names_are_stable(raw in "[a-z][a-z0-9-]{0,45}") {
            if let Ok(n) = HiveName::parse(&raw) {
                prop_assert_eq!(n.as_str(), raw.as_str());
                prop_assert_eq!(HiveName::parse(n.as_str()), Ok(n.clone()));
                prop_assert_eq!(HiveName::from_db_name(&n.db_name()), Some(n));
            }
        }

        /// Nothing that could escape the fleet root is ever accepted.
        #[test]
        fn no_accepted_name_can_walk_a_path(raw in "\\PC{0,48}") {
            if let Ok(n) = HiveName::parse(&raw) {
                let s = n.as_str();
                prop_assert!(!s.contains('/') && !s.contains('\\') && !s.contains('.'));
                prop_assert!(!s.contains(char::is_whitespace));
                prop_assert!(s.len() <= MAX_LEN);
            }
        }
    }
}
