//! Fetching the source recordings listed in `sources.json`, each checked
//! against its git blob hash, so the pack is built from exactly the files
//! it names.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha1::{Digest, Sha1};

#[derive(Deserialize)]
pub struct Sources {
    pub libraries: Vec<Library>,
    pub files: Vec<SourceFile>,
}

#[derive(Deserialize)]
pub struct Library {
    pub repo: String,
    pub url: String,
    pub commit: String,
    pub license: String,
    pub name: String,
}

#[derive(Deserialize)]
pub struct SourceFile {
    pub repo: String,
    pub path: String,
    pub size: u64,
    pub sha: String,
}

pub fn sources() -> Sources {
    serde_json::from_str(include_str!("../sources.json")).expect("sources.json is valid")
}

/// Where a source file is kept under `dir`.
pub fn local(dir: &Path, file: &SourceFile) -> PathBuf {
    dir.join(&file.repo).join(&file.path)
}

/// True if `path` holds exactly the file listed.
fn matches(path: &Path, file: &SourceFile) -> bool {
    let Ok(data) = fs::read(path) else {
        return false;
    };
    if data.len() as u64 != file.size {
        return false;
    }
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", data.len()).as_bytes());
    hasher.update(&data);
    let hash: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    hash == file.sha
}

/// A path's parts percent-encoded for a URL.
fn encode(path: &str) -> String {
    path.split('/')
        .map(|part| {
            part.bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        (byte as char).to_string()
                    }
                    _ => format!("%{byte:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Downloads every listed file not already in `dir`.
pub fn fetch(dir: &Path) -> Result<(), String> {
    let sources = sources();
    let total = sources.files.len();
    let mut fetched = 0;
    for (index, file) in sources.files.iter().enumerate() {
        let path = local(dir, file);
        if matches(&path, file) {
            continue;
        }
        let library = sources
            .libraries
            .iter()
            .find(|library| library.repo == file.repo)
            .ok_or_else(|| format!("no library {}", file.repo))?;
        let url = format!(
            "https://raw.githubusercontent.com/sgossner/{}/{}/{}",
            library.repo,
            library.commit,
            encode(&file.path)
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        let response = ureq::get(&url)
            .call()
            .map_err(|err| format!("{url}: {err}"))?;
        let mut data = Vec::new();
        response
            .into_body()
            .into_reader()
            .read_to_end(&mut data)
            .map_err(|err: io::Error| format!("{url}: {err}"))?;
        let part = path.with_extension("part");
        fs::write(&part, &data).map_err(|err| format!("{}: {err}", part.display()))?;
        fs::rename(&part, &path).map_err(|err| format!("{}: {err}", path.display()))?;
        if !matches(&path, file) {
            return Err(format!("{} does not match its hash", file.path));
        }
        fetched += 1;
        println!("{}/{total} {}", index + 1, file.path);
    }
    println!("{fetched} fetched, {} already here", total - fetched);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_names_its_libraries_and_paths_encode_for_urls() {
        let sources = sources();
        assert_eq!(sources.libraries.len(), 2);
        assert!(sources.libraries.iter().all(|l| l.license == "CC0-1.0"));
        assert!(sources.files.len() > 300);
        assert_eq!(
            encode("Chordophones/Zithers/Grand Piano, Kawai/GPiano_sus_A#3_v1.wav"),
            "Chordophones/Zithers/Grand%20Piano%2C%20Kawai/GPiano_sus_A%233_v1.wav"
        );
    }
}
