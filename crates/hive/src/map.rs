//! The live picture of the fleet.
//!
//! The app hangs a lifecycle hook on the `DatabaseFleet`; every open and close
//! of a hive's database arrives here as a [`MapEvent`], and the front page
//! draws one tile per hive from the result. A tile is lit while its file is
//! open, glows down for a moment after it closes, and stays as a ghost once
//! the hive is deleted.
//!
//! The reducer trusts nothing about order: a close for a hive it never saw
//! open, or an open after a delete (the hive was provisioned again), is folded
//! in rather than refused, because hooks run on blocking threads and the map
//! is a picture, not a ledger.

use std::collections::BTreeMap;

use crate::name::HiveName;

/// How long a tile glows after its file closes.
pub const COOL_MS: u64 = 2_500;

/// What happened to a hive's database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapEvent {
    /// The file exists on disk (seen when the app starts).
    Known(HiveName),
    /// The fleet opened the file.
    Opened(HiveName),
    /// The fleet closed it (idle, least-recently-used, or for a delete).
    Closed(HiveName),
    /// The file was deleted.
    Deleted(HiveName),
}

/// How a tile is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileState {
    /// On disk, closed.
    Cold,
    /// Open now.
    Warm,
    /// Closed within the last [`COOL_MS`].
    Cooling,
    /// Deleted.
    Gone,
}

impl TileState {
    /// The CSS class.
    #[must_use]
    pub const fn class(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
            Self::Cooling => "cooling",
            Self::Gone => "gone",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Raw {
    Closed,
    Open,
    Deleted,
}

/// One hive on the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// The hive.
    pub name: HiveName,
    /// How to draw it now.
    pub state: TileState,
    /// How many times its file has been opened since the app started.
    pub opens: u64,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    raw: Raw,
    opens: u64,
    changed_ms: u64,
}

/// Every hive the app has seen, and what its file is doing.
#[derive(Debug, Clone, Default)]
pub struct FleetMap {
    entries: BTreeMap<HiveName, Entry>,
    opens_total: u64,
    closes_total: u64,
    peak_open: usize,
    open_now: usize,
}

impl FleetMap {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one event in, at `at_ms` (milliseconds on any clock that only goes
    /// forward).
    pub fn apply(&mut self, event: MapEvent, at_ms: u64) {
        let (name, raw) = match event {
            MapEvent::Known(n) => {
                self.entries.entry(n).or_insert(Entry {
                    raw: Raw::Closed,
                    opens: 0,
                    changed_ms: 0,
                });
                return;
            }
            MapEvent::Opened(n) => (n, Raw::Open),
            MapEvent::Closed(n) => (n, Raw::Closed),
            MapEvent::Deleted(n) => (n, Raw::Deleted),
        };
        let entry = self.entries.entry(name).or_insert(Entry {
            raw: Raw::Closed,
            opens: 0,
            changed_ms: at_ms,
        });
        let was_open = entry.raw == Raw::Open;
        match raw {
            Raw::Open => {
                entry.opens += 1;
                self.opens_total += 1;
            }
            Raw::Closed if was_open => self.closes_total += 1,
            Raw::Closed | Raw::Deleted => {}
        }
        entry.raw = raw;
        entry.changed_ms = at_ms;
        match (was_open, raw == Raw::Open) {
            (false, true) => self.open_now += 1,
            (true, false) => self.open_now -= 1,
            _ => {}
        }
        self.peak_open = self.peak_open.max(self.open_now);
    }

    /// The tiles, in name order, as they should be drawn at `now_ms`.
    #[must_use]
    pub fn tiles(&self, now_ms: u64) -> Vec<Tile> {
        self.entries
            .iter()
            .map(|(name, e)| Tile {
                name: name.clone(),
                state: match e.raw {
                    Raw::Open => TileState::Warm,
                    Raw::Deleted => TileState::Gone,
                    Raw::Closed
                        if e.changed_ms > 0 && now_ms.saturating_sub(e.changed_ms) < COOL_MS =>
                    {
                        TileState::Cooling
                    }
                    Raw::Closed => TileState::Cold,
                },
                opens: e.opens,
            })
            .collect()
    }

    /// Hives whose file is open now, by the hooks' account.
    #[must_use]
    pub const fn open_now(&self) -> usize {
        self.open_now
    }

    /// The most files that were ever open at once.
    #[must_use]
    pub const fn peak_open(&self) -> usize {
        self.peak_open
    }

    /// Opens since the app started.
    #[must_use]
    pub const fn opens_total(&self) -> u64 {
        self.opens_total
    }

    /// Closes since the app started.
    #[must_use]
    pub const fn closes_total(&self) -> u64 {
        self.closes_total
    }

    /// Hives on the map, deleted ones included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no hive has been seen.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use proptest::prelude::*;

    fn h(s: &str) -> HiveName {
        HiveName::parse(s).unwrap()
    }

    #[test]
    fn a_hive_warms_then_cools_then_goes_cold() {
        let mut m = FleetMap::new();
        m.apply(MapEvent::Known(h("acme")), 0);
        assert_eq!(m.tiles(10)[0].state, TileState::Cold);
        m.apply(MapEvent::Opened(h("acme")), 100);
        assert_eq!(m.tiles(150)[0].state, TileState::Warm);
        m.apply(MapEvent::Closed(h("acme")), 1_000);
        assert_eq!(m.tiles(1_000 + COOL_MS - 1)[0].state, TileState::Cooling);
        assert_eq!(m.tiles(1_000 + COOL_MS)[0].state, TileState::Cold);
        assert_eq!(m.tiles(0)[0].opens, 1);
    }

    #[test]
    fn deleted_hives_stay_as_ghosts_and_can_come_back() {
        let mut m = FleetMap::new();
        m.apply(MapEvent::Opened(h("acme")), 1);
        m.apply(MapEvent::Closed(h("acme")), 2);
        m.apply(MapEvent::Deleted(h("acme")), 3);
        assert_eq!(m.tiles(9_999)[0].state, TileState::Gone);
        m.apply(MapEvent::Opened(h("acme")), 4);
        assert_eq!(m.tiles(5)[0].state, TileState::Warm);
        assert_eq!(m.open_now(), 1);
    }

    #[test]
    fn a_close_without_an_open_is_folded_in() {
        let mut m = FleetMap::new();
        m.apply(MapEvent::Closed(h("ghost")), 5);
        assert_eq!(m.open_now(), 0);
        assert_eq!(m.closes_total(), 0);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn known_does_not_reset_a_live_tile() {
        let mut m = FleetMap::new();
        m.apply(MapEvent::Opened(h("acme")), 1);
        m.apply(MapEvent::Known(h("acme")), 2);
        assert_eq!(m.tiles(3)[0].state, TileState::Warm);
    }

    fn arb_event() -> impl Strategy<Value = (u8, u8)> {
        (0u8..4, 0u8..8)
    }

    proptest! {
        /// The open count is exactly the set of hives whose last event was an
        /// open, and the peak never undercounts it.
        #[test]
        fn open_now_matches_a_set_model(events in proptest::collection::vec(arb_event(), 0..300)) {
            let mut m = FleetMap::new();
            let mut model = BTreeSet::new();
            for (t, (kind, who)) in events.into_iter().enumerate() {
                let name = h(&format!("h{who}"));
                let ev = match kind {
                    0 => MapEvent::Known(name.clone()),
                    1 => { model.insert(name.clone()); MapEvent::Opened(name) }
                    2 => { model.remove(&name); MapEvent::Closed(name) }
                    _ => { model.remove(&name); MapEvent::Deleted(name) }
                };
                m.apply(ev, t as u64 + 1);
                prop_assert_eq!(m.open_now(), model.len());
                prop_assert!(m.peak_open() >= m.open_now());
                let warm = m.tiles(u64::MAX).iter().filter(|t| t.state == TileState::Warm).count();
                prop_assert_eq!(warm, model.len());
            }
        }
    }
}
