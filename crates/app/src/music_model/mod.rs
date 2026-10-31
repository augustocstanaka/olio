//! The Generator's music model in the app: ACE-Step 1.5, which makes a
//! track from a prompt. Where it lives, what is pinned (uv, the source,
//! the model files), whether it is installed, and the pieces that install
//! it (`install`), run it (`server`) and ask it for a track (`client`).
//!
//! It is installed once, on request, into `MusicModel` in the per-user
//! data folder (`OLIO_MUSIC_MODEL` names another), shared by every
//! library, with the Python it needs; it runs as a helper process on this
//! computer only while a prompted track is made. `OLIO_MUSIC_MODEL_URL`
//! names a music model already running somewhere (tests, development):
//! then nothing is installed or started.

pub mod client;
pub mod install;
pub mod server;

use std::fs;
use std::path::{Path, PathBuf};

use olio_library::Store;
use serde::{Deserialize, Serialize};

pub use crate::fetch::about_gigabytes;

/// The ACE-Step release installed: a tag of github.com/ace-step/ACE-Step-1.5.
pub const VERSION: &str = "0.1.8";
/// The uv release that installs its Python and packages.
pub const UV_VERSION: &str = "0.13.6";
/// The revision of huggingface.co/ACE-Step/Ace-Step1.5 the model files
/// are pinned to.
pub const MODEL_REVISION: &str = "19671f406d603126926c1b7e2adc169acbcade22";
/// What the install takes on disk, with the Python packages: shown before
/// it starts.
pub const DISK_BYTES: u64 = 10_000_000_000;
/// The ACE-Step model the tracks come from, and its code in the source
/// tree (copied over the model's own, as ACE-Step's downloader does).
pub const DIT_MODEL: &str = "acestep-v15-turbo";
const DIT_CODE: &str = "acestep/models/turbo";

/// A file the install needs: where it comes from, where it goes under its
/// folder, how big it is and what it hashes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pinned {
    pub url: &'static str,
    pub path: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

const fn pinned(url: &'static str, path: &'static str, bytes: u64, sha256: &'static str) -> Pinned {
    Pinned {
        url,
        path,
        bytes,
        sha256,
    }
}

/// uv for this computer, if there is one.
pub fn uv_archive() -> Option<Pinned> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(pinned(
            concat!(
                "https://github.com/augustocstanaka/olio/releases/download/music-model-1/",
                "uv-aarch64-apple-darwin.tar.gz"
            ),
            "uv-aarch64-apple-darwin.tar.gz",
            14_235_670,
            "60fbd5c8a1ed681845eea28d9d622fcde03c51e91a43465b64a1e17db1ed3472",
        ))
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some(pinned(
            concat!(
                "https://github.com/augustocstanaka/olio/releases/download/music-model-1/",
                "uv-x86_64-apple-darwin.tar.gz"
            ),
            "uv-x86_64-apple-darwin.tar.gz",
            17_004_934,
            "63ba17fdda55ad1a230d4b37fc6017ffa10c23688c83818bd3bfdf914c652a02",
        ))
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some(pinned(
            concat!(
                "https://github.com/augustocstanaka/olio/releases/download/music-model-1/",
                "uv-x86_64-pc-windows-msvc.zip"
            ),
            "uv-x86_64-pc-windows-msvc.zip",
            15_239_500,
            "a2a8820d5eea32f08ef467ef3b7f66883a9b076d5737a2fc2fbb5f458d1bb50f",
        ))
    } else {
        None
    }
}

/// The ACE-Step source, as GitHub zipped it for the tag, mirrored with the
/// uv archives in Olio's `music-model-1` release.
pub const SOURCE: Pinned = pinned(
    concat!(
        "https://github.com/augustocstanaka/olio/releases/download/music-model-1/",
        "ace-step-v0.1.8.zip"
    ),
    "ace-step-v0.1.8.zip",
    9_032_987,
    "8c39e3e4c7c939ed8d3617e4fbe81585ec869276a228da5d6080bb2c3e4c3db0",
);

/// The model files, from the pinned revision on Hugging Face: the DiT, its
/// VAE and the text encoder. No language model: tracks are made from the
/// prompt alone, which keeps the model under 6 GB of memory.
pub const MODEL_FILES: &[Pinned] = &[
    model(
        "Qwen3-Embedding-0.6B/added_tokens.json",
        707,
        "c0284b582e14987fbd3d5a2cb2bd139084371ed9acbae488829a1c900833c680",
    ),
    model(
        "Qwen3-Embedding-0.6B/chat_template.jinja",
        4116,
        "87a2728cb8dc9fe424d624542f6060ec05a1d285ebbec578bb078900e33396b5",
    ),
    model(
        "Qwen3-Embedding-0.6B/config.json",
        1359,
        "bb23c1607cfe059a58d8f0196cf1cebb52082b1056b8e358a579da80a5759420",
    ),
    model(
        "Qwen3-Embedding-0.6B/merges.txt",
        1_671_853,
        "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5",
    ),
    model(
        "Qwen3-Embedding-0.6B/model.safetensors",
        1_191_586_416,
        "0437e45c94563b09e13cb7a64478fc406947a93cb34a7e05870fc8dcd48e23fd",
    ),
    model(
        "Qwen3-Embedding-0.6B/special_tokens_map.json",
        613,
        "76862e765266b85aa9459767e33cbaf13970f327a0e88d1c65846c2ddd3a1ecd",
    ),
    model(
        "Qwen3-Embedding-0.6B/tokenizer.json",
        11_423_705,
        "def76fb086971c7867b829c23a26261e38d9d74e02139253b38aeb9df8b4b50a",
    ),
    model(
        "Qwen3-Embedding-0.6B/tokenizer_config.json",
        5404,
        "443bfa629eb16387a12edbf92a76f6a6f10b2af3b53d87ba1550adfcf45f7fa0",
    ),
    model(
        "Qwen3-Embedding-0.6B/vocab.json",
        2_776_833,
        "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910",
    ),
    model(
        "acestep-v15-turbo/config.json",
        1968,
        "74745ff704ea49164c3d2d1c99fc0670f3fc635a869f0aec2d1311e6a52d400a",
    ),
    model(
        "acestep-v15-turbo/configuration_acestep_v15.py",
        13_130,
        "b89870c5c7a7ce060eb0bcdbb5ffc86b0b1a324ca325a26be552ea1b42496dc5",
    ),
    model(
        "acestep-v15-turbo/model.safetensors",
        4_787_825_604,
        "3f6e0797fad420a39bd33979eb6e840e30989e34a3794e843d23b60ec6e422d7",
    ),
    model(
        "acestep-v15-turbo/modeling_acestep_v15_turbo.py",
        96_036,
        "c1ab0dd547124fee7ada449b2b86eae8201dc7d15889932643bbb67e3c982444",
    ),
    model(
        "acestep-v15-turbo/silence_latent.pt",
        3_841_215,
        "a778e9dd942f5e8b2c09c55370782d318834432b03dabbcdf70e6ed49ad6358b",
    ),
    model(
        "vae/config.json",
        425,
        "14e019904df567f26df750317a70e2bd08f9f8f3c40ff4a24c97d1cd3f20ccd2",
    ),
    model(
        "vae/diffusion_pytorch_model.safetensors",
        337_431_388,
        "da17edb604c40deaf09e9b24974e590d1ca83a374070e5d0884cfa4bed9a99b0",
    ),
];

/// A model file at the pinned revision; `Pinned::url` knows where.
const fn model(path: &'static str, bytes: u64, sha256: &'static str) -> Pinned {
    pinned("", path, bytes, sha256)
}

impl Pinned {
    /// Where the file is downloaded from: a model file's URL is its path
    /// at the pinned revision on Hugging Face.
    pub fn url(&self) -> String {
        if self.url.is_empty() {
            format!(
                "https://huggingface.co/ACE-Step/Ace-Step1.5/resolve/{MODEL_REVISION}/{}",
                self.path
            )
        } else {
            self.url.to_owned()
        }
    }
}

/// The weights in each model folder, whose presence means the model is
/// there (as ACE-Step itself checks).
pub const WEIGHTS: [&str; 3] = [
    "acestep-v15-turbo/model.safetensors",
    "vae/diffusion_pytorch_model.safetensors",
    "Qwen3-Embedding-0.6B/model.safetensors",
];

/// How many bytes the install downloads.
pub fn download_bytes() -> u64 {
    let models: u64 = MODEL_FILES.iter().map(|file| file.bytes).sum();
    models + SOURCE.bytes + uv_archive().map_or(0, |uv| uv.bytes)
}

/// Where the music model is installed unless `OLIO_MUSIC_MODEL` says
/// otherwise.
pub fn default_dir() -> Option<PathBuf> {
    std::env::var_os("OLIO_MUSIC_MODEL")
        .map(PathBuf::from)
        .or_else(|| Store::default_root().map(|root| root.join("MusicModel")))
}

/// A music model already running, named by `OLIO_MUSIC_MODEL_URL`
/// ("http://127.0.0.1:8001"): used in place of the installed one.
pub fn url_override() -> Option<String> {
    std::env::var("OLIO_MUSIC_MODEL_URL")
        .ok()
        .map(|url| url.trim_end_matches('/').to_owned())
        .filter(|url| !url.is_empty())
}

/// The language model backend ACE-Step is told to use: MLX on Apple
/// silicon, PyTorch elsewhere. (No language model is loaded, but the
/// setting is read at startup.)
pub fn lm_backend() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "mlx"
    } else {
        "pt"
    }
}

/// The folders inside the music model's directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// A relative `root` (`OLIO_MUSIC_MODEL=target/music-model`) is made
    /// absolute: the processes run here start in folders of their own.
    pub fn new(root: &Path) -> Self {
        let root = if root.is_absolute() {
            root.to_owned()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(root))
                .unwrap_or_else(|_| root.to_owned())
        };
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The uv binary.
    pub fn uv(&self) -> PathBuf {
        let name = if cfg!(windows) { "uv.exe" } else { "uv" };
        self.root.join("uv").join(name)
    }

    /// Which uv is installed: `UV_VERSION` once it is.
    pub fn uv_version(&self) -> PathBuf {
        self.root.join("uv").join("VERSION")
    }

    /// The ACE-Step source tree.
    pub fn source(&self) -> PathBuf {
        self.root.join("ace-step")
    }

    /// Which ACE-Step is unpacked: `VERSION` once it is.
    pub fn source_version(&self) -> PathBuf {
        self.source().join("VERSION")
    }

    /// The Python environment uv makes for it.
    pub fn venv(&self) -> PathBuf {
        self.source().join(".venv")
    }

    /// The Python in that environment: the music model's process.
    pub fn venv_python(&self) -> PathBuf {
        if cfg!(windows) {
            self.venv().join("Scripts").join("python.exe")
        } else {
            self.venv().join("bin").join("python")
        }
    }

    /// Where uv keeps the Pythons it installs.
    pub fn python_install(&self) -> PathBuf {
        self.root.join("python")
    }

    /// uv's download cache, removed once the packages are installed.
    pub fn uv_cache(&self) -> PathBuf {
        self.root.join("uv-cache")
    }

    /// The model files: in the source tree, the one place ACE-Step's API
    /// server looks (it ignores `ACESTEP_CHECKPOINTS_DIR`). A new source
    /// keeps them: see `install_source`.
    pub fn checkpoints(&self) -> PathBuf {
        self.source().join("checkpoints")
    }

    /// The DiT model's code in the source tree.
    pub fn dit_code(&self) -> PathBuf {
        self.source().join(DIT_CODE)
    }

    /// Written last: what is installed.
    pub fn marker(&self) -> PathBuf {
        self.root.join("installed.json")
    }

    pub fn install_log(&self) -> PathBuf {
        self.root.join("install.log")
    }

    pub fn server_log(&self) -> PathBuf {
        self.root.join("server.log")
    }

    /// Where a download lands while it is going on.
    pub fn part(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.part"))
    }

    /// Where an archive is unpacked before it is moved into place.
    pub fn unpacking(&self) -> PathBuf {
        self.root.join(".unpacking")
    }
}

/// What is installed, as `installed.json` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    pub version: String,
    pub uv_version: String,
    pub model_revision: String,
}

impl Installed {
    pub fn current() -> Self {
        Self {
            version: VERSION.to_owned(),
            uv_version: UV_VERSION.to_owned(),
            model_revision: MODEL_REVISION.to_owned(),
        }
    }
}

/// The music model installed in `dir`, if the whole of it is: the marker,
/// the Python environment and every model's weights.
pub fn installed(dir: &Path) -> Option<Installed> {
    let layout = Layout::new(dir);
    let marker = fs::read_to_string(layout.marker()).ok()?;
    let installed: Installed = serde_json::from_str(&marker).ok()?;
    if !layout.venv_python().is_file() {
        return None;
    }
    let checkpoints = layout.checkpoints();
    if !WEIGHTS
        .iter()
        .all(|weights| checkpoints.join(weights).is_file())
    {
        return None;
    }
    Some(installed)
}

/// Removes the music model from `dir`: everything in it.
pub fn remove(dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("{}: {err}", dir.display())),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Writes what `installed` looks for into `dir`, with empty weights:
    /// enough for the window to offer a prompt.
    pub(crate) fn write_installed(dir: &Path) {
        let layout = Layout::new(dir);
        let python = layout.venv_python();
        fs::create_dir_all(python.parent().unwrap()).unwrap();
        fs::write(&python, "").unwrap();
        for weights in WEIGHTS {
            let path = layout.checkpoints().join(weights);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "").unwrap();
        }
        fs::write(
            layout.marker(),
            serde_json::to_string(&Installed::current()).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn the_pinned_files_add_up_to_what_the_button_says() {
        let bytes = download_bytes();
        assert!(
            (6_000_000_000..7_000_000_000).contains(&bytes),
            "{bytes} bytes to download"
        );
        assert!(bytes < DISK_BYTES, "the packages take the rest");
        assert_eq!(about_gigabytes(DISK_BYTES), "about 10 GB");
        assert_eq!(MODEL_FILES.len(), 16);
        for file in MODEL_FILES {
            assert_eq!(file.sha256.len(), 64, "{}", file.path);
            assert!(
                file.url()
                    .starts_with("https://huggingface.co/ACE-Step/Ace-Step1.5/resolve/19671f4")
            );
        }
        for weights in WEIGHTS {
            assert!(MODEL_FILES.iter().any(|file| file.path == weights));
        }
        assert!(uv_archive().is_some(), "this is a supported computer");
    }

    #[test]
    fn the_model_is_installed_once_its_marker_python_and_weights_are_there() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(installed(dir.path()), None);
        write_installed(dir.path());
        assert_eq!(installed(dir.path()), Some(Installed::current()));
        let layout = Layout::new(dir.path());
        fs::remove_file(layout.checkpoints().join(WEIGHTS[1])).unwrap();
        assert_eq!(installed(dir.path()), None, "a model missing");
        remove(dir.path()).unwrap();
        assert!(!dir.path().exists());
        remove(dir.path()).unwrap();
        assert!(["mlx", "pt"].contains(&lm_backend()));
    }
}
