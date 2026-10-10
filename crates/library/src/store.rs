//! The library directory on disk (ADR 0003):
//!
//! ```text
//! <root>/library.json      sets and tiles
//! <root>/library.json.bak  the previous library.json
//! <root>/sounds/<hash>.<extension>
//! ```

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::{FORMAT_VERSION, Library, Sound};

const LIBRARY_FILE: &str = "library.json";
const BACKUP_FILE: &str = "library.json.bak";
const SOUNDS_DIR: &str = "sounds";
const LOCK_FILE: &str = "olio.lock";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{} is not a valid Olio library: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("could not convert {}: {message}", path.display())]
    Convert { path: PathBuf, message: String },
    #[error(
        "{} was saved by a newer version of Olio (format {found}; this version reads up to {FORMAT_VERSION})",
        path.display()
    )]
    TooNew { path: PathBuf, found: u32 },
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_owned(),
        source,
    }
}

/// A library directory.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Where the library lives unless told otherwise:
    /// `~/Library/Application Support/Olio` on macOS, `%APPDATA%\Olio` on
    /// Windows.
    pub fn default_root() -> Option<PathBuf> {
        dirs::data_dir().map(|dir| dir.join("Olio"))
    }

    /// Opens (creating if needed) the library directory at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        let sounds = root.join(SOUNDS_DIR);
        fs::create_dir_all(&sounds).map_err(io_error(&sounds))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn library_path(&self) -> PathBuf {
        self.root.join(LIBRARY_FILE)
    }

    /// Reads `library.json`; a directory without one holds the default
    /// library.
    pub fn load(&self) -> Result<Library, StoreError> {
        let path = self.library_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Library::default()),
            Err(err) => return Err(io_error(&path)(err)),
        };

        #[derive(Deserialize)]
        struct Header {
            format_version: u32,
        }
        let parse_error = |source| StoreError::Parse {
            path: path.clone(),
            source,
        };
        let header: Header = serde_json::from_str(&text).map_err(parse_error)?;
        if header.format_version > FORMAT_VERSION {
            return Err(StoreError::TooNew {
                path,
                found: header.format_version,
            });
        }
        let mut library: Library = serde_json::from_str(&text).map_err(parse_error)?;
        library.format_version = FORMAT_VERSION;
        Ok(library)
    }

    /// Writes `library.json` so that a crash never leaves a half-written
    /// file: the new contents go to a temporary file which then replaces
    /// the old one, and the old one is kept as `library.json.bak`.
    pub fn save(&self, library: &Library) -> Result<(), StoreError> {
        let path = self.library_path();
        let temporary = self.root.join(format!("{LIBRARY_FILE}.tmp"));
        let text = serde_json::to_string_pretty(library).map_err(|source| StoreError::Parse {
            path: path.clone(),
            source,
        })?;
        fs::write(&temporary, text + "\n").map_err(io_error(&temporary))?;
        if path.exists() {
            let backup = self.root.join(BACKUP_FILE);
            fs::copy(&path, &backup).map_err(io_error(&backup))?;
        }
        fs::rename(&temporary, &path).map_err(io_error(&path))
    }

    /// Copies an audio file into the library and returns the [`Sound`] that
    /// names the copy. Importing identical bytes again reuses the copy.
    pub fn import_sound(&self, source: &Path) -> Result<Sound, StoreError> {
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        self.import_with(source, &extension, |partial| {
            fs::copy(source, partial)
                .map(drop)
                .map_err(|err| err.to_string())
        })
    }

    /// Imports a file the library keeps in another form, such as the audio
    /// track of a video: `convert` writes the library's copy (with
    /// `extension`) to the path it is given. Importing identical source
    /// bytes again reuses the earlier conversion.
    pub fn import_converted(
        &self,
        source: &Path,
        extension: &str,
        convert: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<Sound, StoreError> {
        self.import_with(source, extension, convert)
    }

    fn import_with(
        &self,
        source: &Path,
        extension: &str,
        write: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<Sound, StoreError> {
        let mut hasher = blake3::Hasher::new();
        let file = File::open(source).map_err(io_error(source))?;
        hasher
            .update_reader(BufReader::new(file))
            .map_err(io_error(source))?;
        let sound = Sound {
            hash: hasher.finalize().to_hex().to_string(),
            extension: extension.to_owned(),
            original_name: source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        let copy = self.sound_path(&sound);
        if !copy.exists() {
            let partial = self
                .root
                .join(SOUNDS_DIR)
                .join(format!("{}.partial", sound.hash));
            if let Err(message) = write(&partial) {
                let _ = fs::remove_file(&partial);
                return Err(StoreError::Convert {
                    path: source.to_owned(),
                    message,
                });
            }
            fs::rename(&partial, &copy).map_err(io_error(&copy))?;
        }
        Ok(sound)
    }

    /// Marks the library as open in the app until the returned [`Lock`] is
    /// dropped, so a script importing into it can tell. Replaces a lock
    /// left behind by a crash.
    pub fn lock(&self) -> Result<Lock, StoreError> {
        let path = self.root.join(LOCK_FILE);
        fs::write(&path, format!("{}\n", std::process::id())).map_err(io_error(&path))?;
        Ok(Lock { path })
    }

    /// The lock file, if the app has the library open (or crashed with it
    /// open).
    pub fn lock_holder(&self) -> Option<PathBuf> {
        let path = self.root.join(LOCK_FILE);
        path.exists().then_some(path)
    }

    /// The library's copy of a sound.
    pub fn sound_path(&self, sound: &Sound) -> PathBuf {
        self.root.join(SOUNDS_DIR).join(sound.file_name())
    }
}

/// Held by the app while it has a library open; removes the lock file when
/// dropped.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
