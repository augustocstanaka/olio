use std::collections::HashSet;

use olio_control::TileKey;
use olio_library::{MAX_COLUMNS, MAX_ROWS, Slot};

fn key(key: char, alt: bool) -> TileKey {
    TileKey { key, alt }
}

#[test]
fn every_slot_has_its_own_key_and_maps_back_to_itself() {
    let mut seen = HashSet::new();
    for row in 0..MAX_ROWS {
        for column in 0..MAX_COLUMNS {
            let slot = Slot::new(row, column);
            let tile_key = TileKey::for_slot(slot).expect("every slot has a key");
            assert!(seen.insert(tile_key), "{tile_key:?} is used twice");
            assert_eq!(tile_key.slot(), Some(slot));
        }
    }
    assert_eq!(seen.len(), 80);
}

#[test]
fn a_three_column_set_plays_like_farrago() {
    let keys: Vec<String> = (0..4)
        .map(|row| {
            (0..3)
                .map(|column| TileKey::for_slot(Slot::new(row, column)).unwrap().label())
                .collect()
        })
        .collect();
    assert_eq!(keys, ["123", "QWE", "ASD", "ZXC"]);
}

#[test]
fn the_lower_four_rows_need_option_or_alt() {
    assert_eq!(TileKey::for_slot(Slot::new(4, 0)), Some(key('1', true)));
    assert_eq!(TileKey::for_slot(Slot::new(7, 9)), Some(key('/', true)));
    assert_eq!(key('q', true).slot(), Some(Slot::new(5, 0)));
    let label = key('q', true).label();
    assert!(label == "⌥Q" || label == "Alt+Q", "{label}");
}

#[test]
fn keys_and_slots_outside_the_grid_map_to_nothing() {
    assert_eq!(key('-', false).slot(), None);
    assert_eq!(key(' ', false).slot(), None);
    assert_eq!(TileKey::for_slot(Slot::new(8, 0)), None);
    assert_eq!(TileKey::for_slot(Slot::new(0, 10)), None);
}

#[test]
fn keys_are_read_as_people_write_them() {
    assert_eq!(TileKey::parse("Q"), Some(key('q', false)));
    assert_eq!(TileKey::parse(" w "), Some(key('w', false)));
    assert_eq!(TileKey::parse("1"), Some(key('1', false)));
    assert_eq!(TileKey::parse(";"), Some(key(';', false)));
    assert_eq!(TileKey::parse("Alt+Q"), Some(key('q', true)));
    assert_eq!(TileKey::parse("option+/"), Some(key('/', true)));
    assert_eq!(TileKey::parse("⌥z"), Some(key('z', true)));
    assert_eq!(TileKey::parse(""), None);
    assert_eq!(TileKey::parse("QW"), None);
    assert_eq!(TileKey::parse("-"), None);
    assert_eq!(TileKey::parse("Ctrl+Q"), None);
}
