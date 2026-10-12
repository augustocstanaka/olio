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
    assert!(tile.normalize, "new tiles normalize");
    assert_eq!(
        (tile.fade_in_seconds, tile.fade_out_seconds, tile.reverb),
        (0.0, 0.0, 0.0)
    );
}

#[test]
fn a_removed_tile_goes_back_to_its_slot_or_the_first_free_one() {
    let mut set = Set::new("Test");
    let id = set.place(sound("a"), Some(Slot::new(1, 2))).unwrap();
    let tile = set.remove_tile(id).unwrap();
    assert_eq!(set.put_back(&tile), Some(Slot::new(1, 2)));
    let back = set.remove_tile(id).unwrap();

    // Its slot was taken meanwhile: the first free one instead.
    set.place(sound("b"), Some(Slot::new(1, 2))).unwrap();
    assert_eq!(set.put_back(&back), Some(Slot::new(0, 0)));
    assert_eq!(set.tile(id).unwrap().slot, Slot::new(0, 0));
}

#[test]
fn an_old_library_without_normalize_turns_it_on() {
    let json = r#"{ "id": "8d6f1f3a-6b7e-4a43-9a51-3f7a2b0c9d11", "slot": { "row": 0, "column": 0 },
        "title": "a", "sound": { "hash": "x", "extension": "wav", "original_name": "a.wav" },
        "color": "purple" }"#;
    let tile: olio_library::Tile = serde_json::from_str(json).unwrap();
    assert!(tile.normalize && tile.trim_silence);
}

#[test]
fn moving_a_tile_to_an_empty_slot_or_onto_another_swaps_them() {
    let mut set = Set::new("Test");
    let a = set.place(sound("a"), Some(Slot::new(0, 0))).unwrap();
    let b = set.place(sound("b"), Some(Slot::new(0, 1))).unwrap();

    assert_eq!(set.move_tile(a, Slot::new(2, 3)), Some(Slot::new(0, 0)));
    assert_eq!(set.tile(a).unwrap().slot, Slot::new(2, 3));

    // Onto b: they swap.
    assert_eq!(set.move_tile(a, Slot::new(0, 1)), Some(Slot::new(2, 3)));
    assert_eq!(set.tile(a).unwrap().slot, Slot::new(0, 1));
    assert_eq!(set.tile(b).unwrap().slot, Slot::new(2, 3));

    // Nowhere, or outside the grid: nothing moves.
    assert_eq!(set.move_tile(a, Slot::new(0, 1)), None);
    assert_eq!(set.move_tile(a, Slot::new(0, 9)), None);
    assert_eq!(set.move_tile(a, Slot::new(MAX_ROWS, 0)), None);
}

#[test]
fn a_duplicate_takes_the_next_free_slot_with_every_setting() {
    let mut set = Set::new("Test");
    let a = set.place(sound("a"), Some(Slot::new(0, 1))).unwrap();
    set.place(sound("b"), Some(Slot::new(0, 2))).unwrap();
    {
        let tile = set.tile_mut(a).unwrap();
        tile.color = TileColor::Teal;
        tile.reverb = 0.3;
        tile.emoji = "🎉".to_owned();
    }
    let copy = set.duplicate(a, None).unwrap();
    let (original, copy) = (set.tile(a).unwrap(), set.tile(copy).unwrap());
    assert_ne!(original.id, copy.id);
    assert_eq!(
        copy.slot,
        Slot::new(0, 3),
        "the first free slot after the original"
    );
    assert_eq!(copy.title, "a copy");
    assert_eq!(copy.sound, original.sound);
    assert_eq!(
        (copy.color, copy.reverb, copy.emoji.as_str()),
        (TileColor::Teal, 0.3, "🎉")
    );

    // Into a chosen slot, or the next free one after it if it is taken.
    let at = set.duplicate(a, Some(Slot::new(3, 0))).unwrap();
    assert_eq!(set.tile(at).unwrap().slot, Slot::new(3, 0));
    let after = set.duplicate(a, Some(Slot::new(0, 2))).unwrap();
    assert_eq!(set.tile(after).unwrap().slot, Slot::new(1, 0));
}

#[test]
fn free_slots_after_wrap_round_and_run_out() {
    let mut set = Set::new("Test");
    set.columns = 2;
    assert_eq!(set.free_slot_after(Slot::new(7, 1)), Some(Slot::new(0, 0)));
    for row in 0..MAX_ROWS {
        for column in 0..2 {
            set.place(sound("x"), Some(Slot::new(row, column))).unwrap();
        }
    }
    assert_eq!(set.free_slot_after(Slot::new(0, 0)), None);
    let id = set.tiles[0].id;
    assert_eq!(set.duplicate(id, None), None, "a full set takes no copy");
}

#[test]
fn a_tile_moved_to_another_set_keeps_its_key_if_it_can() {
    let mut library = Library::default();
    let first = library.sets[0].id;
    let second = library.add_set();
    let a = library
        .set_mut(first)
        .unwrap()
        .place(sound("a"), Some(Slot::new(1, 2)))
        .unwrap();

    assert_eq!(
        library.move_tile_to_set(a, second, None),
        Some((first, Slot::new(1, 2), Slot::new(1, 2)))
    );
    assert!(library.set(first).unwrap().tile(a).is_none());
    assert_eq!(library.tile(a).unwrap().0.id, second);

    // Back again, its slot taken meanwhile: the earliest free one.
    library
        .set_mut(first)
        .unwrap()
        .place(sound("b"), Some(Slot::new(1, 2)))
        .unwrap();
    assert_eq!(
        library.move_tile_to_set(a, first, None),
        Some((second, Slot::new(1, 2), Slot::new(0, 0)))
    );
    // Not into its own set.
    assert_eq!(library.move_tile_to_set(a, first, None), None);
}

#[test]
fn a_tile_copied_to_another_set_keeps_its_title_and_settings() {
    let mut library = Library::default();
    let first = library.sets[0].id;
    let second = library.add_set();
    let a = library
        .set_mut(first)
        .unwrap()
        .place(sound("a"), None)
        .unwrap();
    library.tile_mut(a).unwrap().reverb = 0.5;

    let copy = library.copy_tile_to_set(a, second, None).unwrap();
    let (set, tile) = library.tile(copy).unwrap();
    assert_eq!(set.id, second);
    assert_eq!(
        (tile.title.as_str(), tile.reverb, tile.slot),
        ("a", 0.5, Slot::new(0, 0))
    );
    assert!(
        library.set(first).unwrap().tile(a).is_some(),
        "the original stays"
    );
    assert_eq!(library.copy_tile_to_set(a, first, None), None);
}

#[test]
fn nothing_moves_into_a_full_set() {
    let mut library = Library::default();
    let first = library.sets[0].id;
    let second = library.add_set();
    let a = library
        .set_mut(first)
        .unwrap()
        .place(sound("a"), None)
        .unwrap();
    let full = library.set_mut(second).unwrap();
    full.columns = 1;
    for row in 0..MAX_ROWS {
        full.place(sound("x"), Some(Slot::new(row, 0))).unwrap();
    }
    assert_eq!(library.move_tile_to_set(a, second, None), None);
    assert_eq!(library.copy_tile_to_set(a, second, None), None);
    assert!(library.set(first).unwrap().tile(a).is_some());
}

#[test]
fn a_grid_widens_freely_but_narrows_only_to_its_right_most_tile() {
    let mut set = Set::new("Test");
    assert_eq!(set.min_columns(), 1);
    set.place(sound("a"), Some(Slot::new(2, 2))).unwrap();
    assert_eq!(set.min_columns(), 3);
    assert_eq!(set.set_columns(9), 9);
    assert_eq!(set.set_columns(12), 10, "ten keys in a row at most");
    assert_eq!(set.set_columns(1), 3, "column 3 has a tile");
    assert_eq!(set.columns, 3);
}
