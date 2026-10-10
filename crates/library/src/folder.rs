//! Listing folders the way people expect to read them.

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The files directly inside `folder` that `keep` accepts, in natural
/// alphabetical order. Hidden files (names starting with a dot, such as the
/// `._` files macOS leaves on external drives) are skipped.
pub fn sorted_files(folder: &Path, keep: impl Fn(&Path) -> bool) -> io::Result<Vec<PathBuf>> {
    sorted_entries(folder, |path| path.is_file() && keep(path))
}

/// The folders directly inside `folder`, in natural alphabetical order,
/// hidden ones skipped.
pub fn sorted_subfolders(folder: &Path) -> io::Result<Vec<PathBuf>> {
    sorted_entries(folder, Path::is_dir)
}

fn sorted_entries(folder: &Path, keep: impl Fn(&Path) -> bool) -> io::Result<Vec<PathBuf>> {
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if !name.starts_with('.') && keep(&path) {
            entries.push((name, path));
        }
    }
    entries.sort_by(|(a, _), (b, _)| natural_cmp(a, b));
    Ok(entries.into_iter().map(|(_, path)| path).collect())
}

/// Compares names alphabetically, ignoring case, with runs of digits
/// compared as numbers: "Track 2" comes before "track 10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut left, mut right) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let x = take_number(&mut left);
                let y = take_number(&mut right);
                let ordering = x
                    .trim_start_matches('0')
                    .len()
                    .cmp(&y.trim_start_matches('0').len())
                    .then_with(|| x.trim_start_matches('0').cmp(y.trim_start_matches('0')));
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (Some(x), Some(y)) => {
                let ordering = x.to_lowercase().cmp(y.to_lowercase());
                if ordering != Ordering::Equal {
                    return ordering;
                }
                left.next();
                right.next();
            }
        }
    }
}

fn take_number(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut number = String::new();
    while let Some(&digit) = chars.peek().filter(|c| c.is_ascii_digit()) {
        number.push(digit);
        chars.next();
    }
    number
}
