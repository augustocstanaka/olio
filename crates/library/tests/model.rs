use olio_library::{Library, MAX_ROWS, Set, Slot, Sound, TileColor, columns_to_fit};

fn sound(name: &str) -> Sound {
    Sound {
        hash: format!("hash-of-{name}"),
        extension: "wav".to_owned(),
        original_name: format!("{name}.wav"),
    }
}

#[test]
fn a_new_library_has_one_empty_set() {
    let library = Library::default();
    assert_eq!(library.sets.len(), 1);
    assert!(library.sets[0].tiles.is_empty());
}

#[test]
fn new_sets_get_unused_names() {
    let mut library = Library::default();
    let second = library.add_set();
    assert_eq!(library.set(second).unwrap().name, "Set 2");
    library.set_mut(second).unwrap().name = "Set 3".to_owned();
    let third = library.add_set();
    assert_eq!(library.set(third).unwrap().name, "Set 4");
    assert_eq!(library.remove_set(second).unwrap().name, "Set 3");
    assert_eq!(library.sets.len(), 2);
}

#[test]
fn tiles_fill_the_earliest_free_slot_row_by_row() {
    let mut set = Set::new("Test");
    set.columns = 3;
    let slots: Vec<Slot> = (0..4)
        .map(|i| {
            let id = set.place(sound(&i.to_string()), None).unwrap();
            set.tile(id).unwrap().slot
        })
        .collect();
    assert_eq!(
        slots,
        [
            Slot::new(0, 0),
            Slot::new(0, 1),
            Slot::new(0, 2),
            Slot::new(1, 0)
        ]
    );
}

#[test]
fn a_free_preferred_slot_is_used_and_a_taken_one_is_not() {
    let mut set = Set::new("Test");
    let first = set.place(sound("a"), Some(Slot::new(2, 3))).unwrap();
    assert_eq!(set.tile(first).unwrap().slot, Slot::new(2, 3));
    let second = set.place(sound("b"), Some(Slot::new(2, 3))).unwrap();
    assert_eq!(set.tile(second).unwrap().slot, Slot::new(0, 0));
    let outside = set.place(sound("c"), Some(Slot::new(0, 9))).unwrap();
    assert_eq!(set.tile(outside).unwrap().slot, Slot::new(0, 1));
}

#[test]
fn a_full_set_takes_no_more_tiles() {
    let mut set = Set::new("Test");
    set.columns = 2;
    for i in 0..(usize::from(MAX_ROWS) * 2) {
        assert!(set.place(sound(&i.to_string()), None).is_some());
    }
    assert_eq!(set.first_free_slot(), None);
    assert_eq!(set.place(sound("extra"), None), None);
}

#[test]
fn the_grid_shows_at_least_four_rows_and_every_row_in_use() {
    let mut set = Set::new("Test");
    assert_eq!(set.visible_rows(), 4);
    set.place(sound("a"), Some(Slot::new(5, 0)));
    assert_eq!(set.visible_rows(), 6);
}

#[test]
fn new_tiles_are_titled_after_their_file_and_coloured_by_column() {
    let mut set = Set::new("Test");
    let id = set.place(sound("rock_out"), Some(Slot::new(1, 1))).unwrap();
    let tile = set.tile(id).unwrap();
    assert_eq!(tile.title, "rock out");
    assert_eq!(tile.color, TileColor::Pink);
    assert!(!tile.looping);
}

#[test]
fn tiles_are_found_and_removed_by_id() {
    let mut library = Library::default();
    let set = library.sets[0].id;
    let id = library
        .set_mut(set)
        .unwrap()
        .place(sound("a"), None)
        .unwrap();
    assert_eq!(library.tile(id).unwrap().0.id, set);
    library.tile_mut(id).unwrap().looping = true;
    let removed = library.set_mut(set).unwrap().remove_tile(id).unwrap();
    assert!(removed.looping);
    assert!(library.tile(id).is_none());
}

#[test]
fn set_names_are_made_unique_by_numbering() {
    let mut library = Library::default();
    assert_eq!(library.unique_set_name("SFX"), "SFX");
    library.sets[0].name = "SFX".to_owned();
    assert_eq!(library.unique_set_name("SFX"), "SFX 2");
    library.add_set();
    library.sets[1].name = "SFX 2".to_owned();
    assert_eq!(library.unique_set_name("SFX"), "SFX 3");
}

#[test]
fn imported_sets_are_sized_to_about_four_rows() {
    assert_eq!(columns_to_fit(0), 4);
    assert_eq!(columns_to_fit(12), 4);
    assert_eq!(columns_to_fit(17), 5);
    assert_eq!(columns_to_fit(40), 10);
    assert_eq!(columns_to_fit(500), 10);
}

#[test]
fn new_tiles_trim_silence_and_have_no_fades_or_reverb() {
    let mut set = Set::new("Test");
    let id = set.place(sound("a"), None).unwrap();
    let tile = set.tile(id).unwrap();
    assert!(tile.trim_silence);
    assert_eq!(
        (tile.fade_in_seconds, tile.fade_out_seconds, tile.reverb),
        (0.0, 0.0, 0.0)
    );
}
