//! The Generator's recorded instruments in the app: where the instrument
//! pack lives, downloading and installing it, and loading the instruments
//! the music uses, each on a thread of its own.
//!
//! The pack is shared by every library: it lives in `Instruments` in the
//! per-user data folder (`OLIO_INSTRUMENTS` names another). Built by
//! `cargo xtask instruments`; published as a release of the repository.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use olio_audio::ambience::Instrument;
use olio_audio::instruments::{Multisample, Pack};
use olio_library::Store;
use sha2::{Digest, Sha256};

/// The published pack: where it is, what it hashes to, how big it is.
pub const PACK_URL: &str = "https://github.com/augustocstanaka/olio/releases/download/instruments-1/olio-instruments-1.zip";
pub const PACK_SHA256: &str = "9b6df58d19982b4beecc8dfd2c93c47ff2f6b1e92bf1ff44ca848efd1db606c4";
pub const PACK_BYTES: u64 = 71_276_756;
/// The pack's folder, inside the zip and once installed.
const PACK_NAME: &str = "olio-instruments";

/// Where the pack is installed unless `OLIO_INSTRUMENTS` says otherwise.
pub fn default_dir() -> Option<PathBuf> {
    std::env::var_os("OLIO_INSTRUMENTS")
        .map(PathBuf::from)
        .or_else(|| Store::default_root().map(|root| root.join("Instruments")))
}

/// The pack installed in `dir`, if there is one Olio can read.
pub fn installed(dir: &Path) -> Option<Pack> {
    Pack::open(&dir.join(PACK_NAME)).ok()
}

/// "71 MB".
pub fn megabytes(bytes: u64) -> String {
    format!("{} MB", (bytes as f64 / 1e6).round())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Installs the zipped pack at `zip_path` into `dir`, checking its
/// SHA-256 first when one is given; a pack already there is replaced.
pub fn install_zip(zip_path: &Path, sha256: Option<&str>, dir: &Path) -> Result<(), String> {
    let data = fs::read(zip_path).map_err(|err| format!("{}: {err}", zip_path.display()))?;
    if let Some(expected) = sha256 {
        let actual = hex(&Sha256::digest(&data));
        if actual != expected {
            return Err("the download is damaged (its checksum is wrong)".to_owned());
        }
    }
    let unpacking = dir.join(".unpacking");
    let _ = fs::remove_dir_all(&unpacking);
    fs::create_dir_all(&unpacking).map_err(|err| format!("{}: {err}", unpacking.display()))?;
    let mut archive =
        zip::ZipArchive::new(io::Cursor::new(data)).map_err(|err| format!("not a pack: {err}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("not a pack: {err}"))?;
        // Only names that stay inside the folder.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let path = unpacking.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&path).map_err(|err| format!("{}: {err}", path.display()))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|err| format!("not a pack: {err}"))?;
        fs::write(&path, contents).map_err(|err| format!("{}: {err}", path.display()))?;
    }
    let unpacked = unpacking.join(PACK_NAME);
    Pack::open(&unpacked)?;
    let target = dir.join(PACK_NAME);
    let _ = fs::remove_dir_all(&target);
    fs::rename(&unpacked, &target).map_err(|err| format!("{}: {err}", target.display()))?;
    let _ = fs::remove_dir_all(&unpacking);
    Ok(())
}

/// Downloads `url` to `path`, telling `progress` how far, 0 to 1; stops
/// (with `None`) when it says to.
fn download(
    url: &str,
    path: &Path,
    mut progress: impl FnMut(f32) -> bool,
) -> Result<Option<()>, String> {
    let response = ureq::get(url).call().map_err(|err| err.to_string())?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(PACK_BYTES);
    let mut body = response.into_body().into_reader();
    let mut file = fs::File::create(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let read = body.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            return Ok(Some(()));
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("{}: {err}", path.display()))?;
        done += read as u64;
        if !progress((done as f32 / total.max(1) as f32).min(1.0)) {
            return Ok(None);
        }
    }
}

/// Downloads the published pack into `dir` and installs it: `true` once
/// installed, `false` if `progress` (told how far, 0 to 1) said to stop.
pub fn download_and_install(dir: &Path, progress: impl FnMut(f32) -> bool) -> Result<bool, String> {
    fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let part = dir.join("download.part");
    let result = match download(PACK_URL, &part, progress) {
        Ok(Some(())) => install_zip(&part, Some(PACK_SHA256), dir).map(|()| true),
        Ok(None) => Ok(false),
        Err(err) => Err(err),
    };
    let _ = fs::remove_file(&part);
    result
}

/// The pack being downloaded and installed, on a thread of its own.
pub struct Download {
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    done: mpsc::Receiver<Result<bool, String>>,
}

impl Download {
    /// Starts downloading the published pack into `dir`.
    pub fn start(ctx: egui::Context, dir: PathBuf) -> Self {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, done) = mpsc::channel();
        let (shared, stop) = (Arc::clone(&progress), Arc::clone(&cancel));
        let spawned = thread::Builder::new()
            .name("olio-instruments".to_owned())
            .spawn(move || {
                let mut shown = Instant::now();
                let result = download_and_install(&dir, |done| {
                    shared.store(done.to_bits(), Ordering::Relaxed);
                    if shown.elapsed() > Duration::from_millis(100) {
                        shown = Instant::now();
                        ctx.request_repaint();
                    }
                    !stop.load(Ordering::Relaxed)
                });
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        if let Err(err) = spawned {
            eprintln!("olio: could not start the download: {err}");
        }
        Self {
            progress,
            cancel,
            done,
        }
    }

    /// 0 to 1.
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Once finished: installed (`true`), cancelled (`false`), or what
    /// went wrong.
    pub fn poll(&self) -> Option<Result<bool, String>> {
        match self.done.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the download stopped unexpectedly".to_owned()))
            }
        }
    }
}

/// An instrument being loaded from the pack, on a thread of its own.
pub struct Loading {
    pub instrument: Instrument,
    done: mpsc::Receiver<Result<Option<Multisample>, String>>,
}

impl Loading {
    pub fn start(ctx: egui::Context, pack: Pack, instrument: Instrument) -> Self {
        let (sender, done) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("olio-instrument-load".to_owned())
            .spawn(move || {
                let _ = sender.send(pack.load(instrument));
                ctx.request_repaint();
            });
        if let Err(err) = spawned {
            eprintln!("olio: could not load an instrument: {err}");
        }
        Self { instrument, done }
    }

    pub fn poll(&self) -> Option<Result<Option<Multisample>, String>> {
        match self.done.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("loading stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use olio_audio::instruments::{
        InstrumentManifest, MANIFEST, Manifest, PACK_VERSION, ZoneManifest,
    };
    use olio_audio::{Clip, WavFormat};

    /// Writes a small pack to `dir/olio-instruments`: a harp of two sine
    /// notes.
    pub(crate) fn write_pack(dir: &Path) {
        let pack = dir.join(PACK_NAME);
        fs::create_dir_all(pack.join("harp")).unwrap();
        let rate = 44_100;
        for (file, hz) in [("A3", 220.0f32), ("A5", 880.0)] {
            let samples: Vec<f32> = (0..rate)
                .flat_map(|i| {
                    let t = i as f32 / rate as f32;
                    let s = (t * hz * std::f32::consts::TAU).sin() * 0.4 * (-t * 3.0).exp();
                    [s, s]
                })
                .collect();
            Clip::new(rate, 2, samples)
                .write_wav(&pack.join(format!("harp/{file}.wav")), WavFormat::Int16)
                .unwrap();
        }
        let zone = |file: &str, root: u8, low: u8, high: u8| ZoneManifest {
            file: file.to_owned(),
            root,
            low,
            high,
            velocity: (0.0, 1.0),
            looped: None,
            gain: 1.0,
            pitched: true,
            tune: 0.0,
        };
        let manifest = Manifest {
            version: PACK_VERSION,
            instruments: vec![InstrumentManifest {
                instrument: Instrument::Harp,
                attack: 0.001,
                release: 0.3,
                gain: 1.0,
                zones: vec![
                    zone("harp/A3.wav", 57, 0, 69),
                    zone("harp/A5.wav", 81, 70, 127),
                ],
            }],
        };
        fs::write(
            pack.join(MANIFEST),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    /// Zips the pack in `from` as a release would.
    fn zip_pack(from: &Path, zip_path: &Path) {
        let file = fs::File::create(zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for name in ["pack.json", "harp/A3.wav", "harp/A5.wav"] {
            zip.start_file(format!("{PACK_NAME}/{name}"), options)
                .unwrap();
            zip.write_all(&fs::read(from.join(PACK_NAME).join(name)).unwrap())
                .unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn a_zipped_pack_installs_when_its_checksum_is_right() {
        let built = tempfile::tempdir().unwrap();
        write_pack(built.path());
        let zip_path = built.path().join("pack.zip");
        zip_pack(built.path(), &zip_path);
        let sha = hex(&Sha256::digest(fs::read(&zip_path).unwrap()));

        let dir = tempfile::tempdir().unwrap();
        assert!(installed(dir.path()).is_none());
        let wrong = "0".repeat(64);
        assert!(
            install_zip(&zip_path, Some(&wrong), dir.path())
                .unwrap_err()
                .contains("damaged")
        );
        assert!(installed(dir.path()).is_none(), "nothing half-installed");

        install_zip(&zip_path, Some(&sha), dir.path()).unwrap();
        let pack = installed(dir.path()).expect("installed");
        assert_eq!(pack.instruments().collect::<Vec<_>>(), [Instrument::Harp]);
        assert!(!dir.path().join(".unpacking").exists());
        // Again, over the top of itself.
        install_zip(&zip_path, None, dir.path()).unwrap();
        assert!(installed(dir.path()).is_some());
        assert_eq!(megabytes(PACK_BYTES), "71 MB");
    }
}
