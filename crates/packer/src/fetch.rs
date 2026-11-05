//! Fetching the source recordings listed in `sources.json`, each checked
//! against its git blob hash, so the pack is built from exactly the files
//! it names.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::recipes::RECIPES;

#[derive(Deserialize)]
pub struct Sources {
    pub libraries: Vec<Library>,
    pub files: Vec<SourceFile>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Library {
    pub repo: String,
    pub url: String,
    pub commit: String,
    pub license: String,
    pub name: String,
    /// Who to credit.
    pub credit: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SourceFile {
    pub repo: String,
    pub path: String,
    pub size: u64,
    pub sha: String,
}

/// `sources.json` as written.
#[derive(Serialize)]
struct Listing<'a> {
    about: &'a str,
    libraries: &'a [Library],
    files: &'a [SourceFile],
}

const ABOUT: &str = "Source recordings for Olio's instrument pack, pinned to commits of the Versilian Studios libraries (CC0 1.0). Written by `olio-packer pin` from the recipes: the files they take, each with its git blob hash.";

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

/// A directory listing of a library at its pinned commit, as GitHub's
/// API gives it.
#[derive(Deserialize)]
struct Tree {
    truncated: bool,
    tree: Vec<TreeEntry>,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    #[serde(default)]
    size: u64,
}

/// Every recording of every library, at the commits `sources.json` pins,
/// read from GitHub's tree listing.
fn listing(libraries: &[Library]) -> Result<Vec<SourceFile>, String> {
    let mut files = Vec::new();
    for library in libraries {
        let url = format!(
            "https://api.github.com/repos/sgossner/{}/git/trees/{}?recursive=1",
            library.repo, library.commit
        );
        let response = ureq::get(&url)
            .header("User-Agent", "olio-packer")
            .call()
            .map_err(|err| format!("{url}: {err}"))?;
        let mut data = Vec::new();
        response
            .into_body()
            .into_reader()
            .read_to_end(&mut data)
            .map_err(|err: io::Error| format!("{url}: {err}"))?;
        let tree: Tree = serde_json::from_slice(&data).map_err(|err| format!("{url}: {err}"))?;
        if tree.truncated {
            return Err(format!("{}: the listing was cut short", library.repo));
        }
        files.extend(
            tree.tree
                .into_iter()
                .filter(|entry| entry.kind == "blob" && entry.path.ends_with(".wav"))
                .map(|entry| SourceFile {
                    repo: library.repo.clone(),
                    path: entry.path,
                    size: entry.size,
                    sha: entry.sha,
                }),
        );
    }
    Ok(files)
}

/// Rewrites `sources.json` (in the packer's source folder) to list the
/// files the recipes take from the libraries' pinned commits, with what
/// each recipe takes and the download it makes.
pub fn pin() -> Result<(), String> {
    let libraries = sources().libraries;
    let everything = Sources {
        files: listing(&libraries)?,
        libraries: libraries.clone(),
    };
    let mut chosen: Vec<SourceFile> = Vec::new();
    let mut total = 0;
    for recipe in RECIPES {
        let mut bytes = 0;
        let takes = crate::build::gather_thinned(recipe, &everything, Path::new(""), true)?;
        for (path, _) in &takes {
            let file = everything
                .files
                .iter()
                .find(|file| &local(Path::new(""), file) == path)
                .ok_or_else(|| format!("{} is not listed", path.display()))?;
            bytes += file.size;
            if !chosen
                .iter()
                .any(|have| have.repo == file.repo && have.path == file.path)
            {
                chosen.push(file.clone());
            }
        }
        println!(
            "{:?} {}: {} recordings, {:.1} MB",
            recipe.instrument,
            recipe.name,
            takes.len(),
            bytes as f64 / 1e6
        );
        total += bytes;
    }
    chosen.sort_by(|a, b| (&a.repo, &a.path).cmp(&(&b.repo, &b.path)));
    let listing = Listing {
        about: ABOUT,
        libraries: &libraries,
        files: &chosen,
    };
    let mut text = Vec::new();
    let format = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut text, format);
    listing
        .serialize(&mut serializer)
        .map_err(|err| err.to_string())?;
    text.push(b'\n');
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("sources.json");
    fs::write(&path, text).map_err(|err| format!("{}: {err}", path.display()))?;
    println!(
        "{} files, {:.0} MB to download (some used by several voices: {:.0} MB counted twice)",
        chosen.len(),
        chosen.iter().map(|file| file.size).sum::<u64>() as f64 / 1e6,
        (total - chosen.iter().map(|file| file.size).sum::<u64>()) as f64 / 1e6
    );
    Ok(())
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
