//! Translating egui keys into the control crate's key names.

use egui::Key;

/// The keymap character for a key that can play a tile (see
/// `olio_control::keymap`).
pub fn tile_char(key: Key) -> Option<char> {
    Some(match key {
        Key::Num1 => '1',
        Key::Num2 => '2',
        Key::Num3 => '3',
        Key::Num4 => '4',
        Key::Num5 => '5',
        Key::Num6 => '6',
        Key::Num7 => '7',
        Key::Num8 => '8',
        Key::Num9 => '9',
        Key::Num0 => '0',
        Key::Q => 'q',
        Key::W => 'w',
        Key::E => 'e',
        Key::R => 'r',
        Key::T => 't',
        Key::Y => 'y',
        Key::U => 'u',
        Key::I => 'i',
        Key::O => 'o',
        Key::P => 'p',
        Key::A => 'a',
        Key::S => 's',
        Key::D => 'd',
        Key::F => 'f',
        Key::G => 'g',
        Key::H => 'h',
        Key::J => 'j',
        Key::K => 'k',
        Key::L => 'l',
        Key::Semicolon => ';',
        Key::Z => 'z',
        Key::X => 'x',
        Key::C => 'c',
        Key::V => 'v',
        Key::B => 'b',
        Key::N => 'n',
        Key::M => 'm',
        Key::Comma => ',',
        Key::Period => '.',
        Key::Slash => '/',
        _ => return None,
    })
}

/// 1 to 9 for the number-row keys, for the set shortcuts.
pub fn set_number(key: Key) -> Option<usize> {
    match tile_char(key)? {
        digit @ '1'..='9' => digit.to_digit(10).map(|digit| digit as usize),
        _ => None,
    }
}

/// The key that selects the `number`th set together with Command (Ctrl).
pub fn set_key(number: usize) -> Option<Key> {
    [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ]
    .get(number.checked_sub(1)?)
    .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use olio_control::keymap::KEY_ROWS;

    #[test]
    fn every_key_of_the_keymap_has_an_egui_key() {
        for character in KEY_ROWS.iter().flat_map(|row| row.chars()) {
            assert!(
                Key::ALL
                    .iter()
                    .any(|&key| tile_char(key) == Some(character)),
                "no egui key for {character:?}"
            );
        }
    }

    #[test]
    fn set_shortcuts_are_the_digits_one_to_nine() {
        assert_eq!(set_number(Key::Num1), Some(1));
        assert_eq!(set_number(Key::Num9), Some(9));
        assert_eq!(set_number(Key::Num0), None);
        assert_eq!(set_number(Key::Q), None);
        assert_eq!(set_key(3), Some(Key::Num3));
        assert_eq!(set_key(0), None);
        assert_eq!(set_key(10), None);
    }
}
