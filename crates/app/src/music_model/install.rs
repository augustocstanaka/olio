//! Installing the music model: uv, then the ACE-Step source, then its
//! Python and packages (`uv sync`), then the model files, each phase
//! skipped when it is already done, so a stopped install goes on from
//! where it was. On a thread of its own in the app ([`Install`]); on the
//! calling thread for `olio music-model install`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::{
    DIT_MODEL, Installed, Layout, MODEL_FILES, Pinned, SOURCE, UV_VERSION, VERSION, uv_archive,
};
use crate::fetch;

/// Where the install has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Uv,
    Source,
    /// `uv sync`: Python and the packages.
    Sync,
    Models,
    Finish,
}

impl Phase {
    /// "downloading the models", for the progress bar.
    pub fn words(self) -> &'static str {
        match self {
            Self::Uv => "downloading uv",
            Self::Source => "downloading the source",
            Self::Sync => "installing Python and its packages",
            Self::Models => "downloading the models",
            Self::Finish => "finishing",
        }
    }

    fn index(self) -> u8 {
        match self {
            Self::Uv => 0,
            Self::Source => 1,
            Self::Sync => 2,
            Self::Models => 3,
            Self::Finish => 4,
        }
    }

    fn from_index(index: u8) -> Self {
        match index {
            0 => Self::Uv,
            1 => Self::Source,
            2 => Self::Sync,
            3 => Self::Models,
            _ => Self::Finish,
        }
    }
}

/// A file to download: where from, where to (a name under the root for an
/// archive, a path under `checkpoints/` for a model file), how big, what
/// it hashes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub url: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl Download {
    fn of(pinned: &Pinned) -> Self {
        Self {
            url: pinned.url(),
            path: pinned.path.to_owned(),
            bytes: pinned.bytes,
            sha256: pinned.sha256.to_owned(),
        }
    }
}

/// Where the install's files come from: the published ones, or a test's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sources {
    /// uv for this computer; `None` where there is none.
    pub uv: Option<Download>,
    pub source: Download,
    pub models: Vec<Download>,
}

impl Sources {
    pub fn published() -> Self {
        Self {
            uv: uv_archive().as_ref().map(Download::of),
            source: Download::of(&SOURCE),
            models: MODEL_FILES.iter().map(Download::of).collect(),
        }
    }
}

/// How the Python packages are installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync {
    /// `uv sync` in the source tree.
    Uv,
    /// Not at all (tests).
    Skip,
}

/// The whole install into `dir`, resumable: `report` is told the phase
/// and, where known, how far it is, 0 to 1, and stops the install by
/// returning false (`Ok(false)`). What was downloaded stays for next time.
pub fn run(
    dir: &Path,
    sources: &Sources,
    sync: Sync,
    mut report: impl FnMut(Phase, Option<f32>) -> bool,
) -> Result<bool, String> {
    let layout = Layout::new(dir);
    fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let _ = fs::remove_file(layout.marker());

    // uv.
    if let Some(uv) = &sources.uv
        && !(layout.uv().is_file() && reads(&layout.uv_version(), UV_VERSION))
    {
        let part = layout.part(&uv.path);
        if download(uv, &part, |done| report(Phase::Uv, Some(done)))?.is_none() {
            return Ok(false);
        }
        install_uv(&layout, &part, &uv.path)?;
        let _ = fs::remove_file(&part);
    } else if sources.uv.is_none() && sync == Sync::Uv {
        return Err("the music model is not available for this computer".to_owned());
    }

    // The source.
    if !reads(&layout.source_version(), VERSION) {
        let part = layout.part(&sources.source.path);
        if download(&sources.source, &part, |done| {
            report(Phase::Source, Some(done))
        })?
        .is_none()
        {
            return Ok(false);
        }
        install_source(&layout, &part)?;
        let _ = fs::remove_file(&part);
    }

    // Python and the packages.
    if !report(Phase::Sync, None) {
        return Ok(false);
    }
    if sync == Sync::Uv && !uv_sync(&layout, || report(Phase::Sync, None))? {
        return Ok(false);
    }

    // The models.
    let checkpoints = layout.checkpoints();
    let missing: Vec<&Download> = sources
        .models
        .iter()
        .filter(|model| !model_present(&checkpoints, model))
        .collect();
    let total: u64 = missing.iter().map(|model| model.bytes).sum::<u64>().max(1);
    let mut before = 0u64;
    for model in missing {
        let part = layout.part(&model.path.replace(['/', '\\'], "-"));
        let overall = |done: f32| (before as f32 + done * model.bytes as f32) / total as f32;
        if download(model, &part, |done| {
            report(Phase::Models, Some(overall(done)))
        })?
        .is_none()
        {
            return Ok(false);
        }
        let dest = checkpoints.join(&model.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        fs::rename(&part, &dest).map_err(|err| format!("{}: {err}", dest.display()))?;
        fs::write(checked_marker(&dest), &model.sha256)
            .map_err(|err| format!("{}: {err}", dest.display()))?;
        before += model.bytes;
    }

    // Finish: the model's code from the source tree, then the marker.
    if !report(Phase::Finish, None) {
        return Ok(false);
    }
    copy_model_code(&layout)?;
    let marker =
        serde_json::to_string_pretty(&Installed::current()).map_err(|err| err.to_string())?;
    fs::write(layout.marker(), marker)
        .map_err(|err| format!("{}: {err}", layout.marker().display()))?;
    Ok(true)
}

/// True if the file holds `text`.
fn reads(path: &Path, text: &str) -> bool {
    fs::read_to_string(path).is_ok_and(|contents| contents.trim() == text)
}

/// Beside a model file once its hash was checked.
fn checked_marker(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".sha256");
    dest.with_file_name(name)
}

/// True if the model file is there, whole and checked.
fn model_present(checkpoints: &Path, model: &Download) -> bool {
    let dest = checkpoints.join(&model.path);
    fs::metadata(&dest).is_ok_and(|meta| meta.len() == model.bytes)
        && reads(&checked_marker(&dest), &model.sha256)
}

/// Downloads to `part`, going on from what is there, and checks the size
/// and hash: a wrong one removes the file and fails.
fn download(
    download: &Download,
    part: &Path,
    progress: impl FnMut(f32) -> bool,
) -> Result<Option<()>, String> {
    let fetched = fetch::download(&download.url, part, true, Some(download.bytes), progress)?;
    let Some(fetched) = fetched else {
        return Ok(None);
    };
    if fetched.bytes != download.bytes || fetched.sha256 != download.sha256 {
        let _ = fs::remove_file(part);
        return Err(format!(
            "the download of {} is damaged (its checksum is wrong)",
            download.path
        ));
    }
    Ok(Some(()))
}

/// Unpacks uv from its archive (a tarball on macOS, a zip on Windows) into
/// place and notes its version.
fn install_uv(layout: &Layout, archive: &Path, name: &str) -> Result<(), String> {
    let unpacking = layout.unpacking();
    let _ = fs::remove_dir_all(&unpacking);
    fs::create_dir_all(&unpacking).map_err(|err| format!("{}: {err}", unpacking.display()))?;
    let binary = if cfg!(windows) { "uv.exe" } else { "uv" };
    let unpacked = unpacking.join(binary);
    if name.ends_with(".zip") {
        unzip(archive, &unpacking, 0, Some(binary))?;
    } else {
        untar(archive, &unpacked, binary)?;
    }
    if !unpacked.is_file() {
        return Err("the uv archive holds no uv".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&unpacked, fs::Permissions::from_mode(0o755))
            .map_err(|err| format!("{}: {err}", unpacked.display()))?;
    }
    let target = layout.uv();
    let dir = target.parent().ok_or("no uv folder")?;
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    fs::rename(&unpacked, &target).map_err(|err| format!("{}: {err}", target.display()))?;
    fs::write(layout.uv_version(), UV_VERSION).map_err(|err| err.to_string())?;
    let _ = fs::remove_dir_all(&unpacking);
    Ok(())
}

/// Unpacks the source zip into place (its top folder dropped) and notes
/// its version. A source tree already there goes, with its environment.
fn install_source(layout: &Layout, archive: &Path) -> Result<(), String> {
    let unpacking = layout.unpacking();
    let _ = fs::remove_dir_all(&unpacking);
    fs::create_dir_all(&unpacking).map_err(|err| format!("{}: {err}", unpacking.display()))?;
    unzip(archive, &unpacking, 1, None)?;
    if !unpacking.join("pyproject.toml").is_file() {
        return Err("the source archive is not ACE-Step".to_owned());
    }
    let target = layout.source();
    // The model files live in the tree: a newer tree takes them over.
    let checkpoints = layout.checkpoints();
    if checkpoints.is_dir() {
        let kept = unpacking.join("checkpoints");
        fs::rename(&checkpoints, &kept).map_err(|err| format!("{}: {err}", kept.display()))?;
    }
    let _ = fs::remove_dir_all(&target);
    fs::rename(&unpacking, &target).map_err(|err| format!("{}: {err}", target.display()))?;
    fs::write(layout.source_version(), VERSION).map_err(|err| err.to_string())?;
    Ok(())
}

/// Unpacks `archive` into `into`, dropping `strip` leading folders from
/// every name, only `only` (a file name) when given. Names that would
/// leave the folder are skipped.
pub(crate) fn unzip(
    archive: &Path,
    into: &Path,
    strip: usize,
    only: Option<&str>,
) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|err| format!("{}: {err}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file))
        .map_err(|err| format!("not an archive: {err}"))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|err| format!("not an archive: {err}"))?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let relative: PathBuf = relative.components().skip(strip).collect();
        if relative.as_os_str().is_empty() {
            continue;
        }
        if let Some(only) = only
            && relative.file_name().is_none_or(|name| name != only)
        {
            continue;
        }
        let path = into.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&path).map_err(|err| format!("{}: {err}", path.display()))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        let mut out =
            fs::File::create(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        io::copy(&mut entry, &mut out).map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(())
}

/// Takes the file called `name` out of a gzipped tarball into `into`.
fn untar(archive: &Path, into: &Path, name: &str) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|err| format!("{}: {err}", archive.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(file)));
    let entries = tar
        .entries()
        .map_err(|err| format!("not an archive: {err}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|err| format!("not an archive: {err}"))?;
        let path = entry.path().map_err(|err| err.to_string())?.into_owned();
        if entry.header().entry_type().is_file() && path.file_name().is_some_and(|f| f == name) {
            let mut out =
                fs::File::create(into).map_err(|err| format!("{}: {err}", into.display()))?;
            io::copy(&mut entry, &mut out).map_err(|err| format!("{}: {err}", into.display()))?;
            return Ok(());
        }
    }
    Err(format!("the archive holds no {name}"))
}

/// `uv sync` in the source tree, its output in `install.log`: `Ok(false)`
/// if `keep_going` said to stop (the process is killed). uv installs its
/// own Python (the source wants 3.11 or 3.12) under the music model's
/// folder, and its cache goes once the packages are in.
fn uv_sync(layout: &Layout, mut keep_going: impl FnMut() -> bool) -> Result<bool, String> {
    let log_path = layout.install_log();
    let log =
        fs::File::create(&log_path).map_err(|err| format!("{}: {err}", log_path.display()))?;
    let errors = log.try_clone().map_err(|err| err.to_string())?;
    let mut command = Command::new(layout.uv());
    command
        .args([
            "sync",
            "--frozen",
            "--no-dev",
            "--no-progress",
            "--color",
            "never",
        ])
        .current_dir(layout.source())
        .env("UV_PROJECT_ENVIRONMENT", layout.venv())
        .env("UV_CACHE_DIR", layout.uv_cache())
        .env("UV_PYTHON_INSTALL_DIR", layout.python_install())
        .env("UV_PYTHON_PREFERENCE", "only-managed")
        .env("UV_NO_PROGRESS", "1")
        .env("UV_HTTP_TIMEOUT", "600")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(errors));
    quiet(&mut command);
    let mut child = command
        .spawn()
        .map_err(|err| format!("could not run uv: {err}"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                return Err(format!(
                    "uv sync failed ({status}): {}\nSee {}",
                    log_tail(&log_path),
                    log_path.display()
                ));
            }
            Ok(None) => {}
            Err(err) => return Err(format!("uv sync: {err}")),
        }
        if !keep_going() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(200));
    }
    let _ = fs::remove_dir_all(layout.uv_cache());
    Ok(true)
}

/// The last lines of a log, for an error message.
fn log_tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let from = lines.len().saturating_sub(12);
    lines[from..].join("\n")
}

/// No console window for a child process on Windows.
pub(crate) fn quiet(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command;
}

/// Copies the DiT model's code from the source tree over the model's own,
/// as ACE-Step's downloader does, so the two are of the same release.
fn copy_model_code(layout: &Layout) -> Result<(), String> {
    let code = layout.dit_code();
    let Ok(entries) = fs::read_dir(&code) else {
        return Ok(());
    };
    let into = layout.checkpoints().join(DIT_MODEL);
    fs::create_dir_all(&into).map_err(|err| format!("{}: {err}", into.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.extension().is_some_and(|ext| ext == "py") && name != "__init__.py" {
            fs::copy(&path, into.join(&name))
                .map_err(|err| format!("{}: {err}", path.display()))?;
        }
    }
    Ok(())
}

/// The music model being installed, on a thread of its own.
pub struct Install {
    phase: Arc<AtomicU8>,
    /// How far through the phase, as `f32` bits; NaN when unknown.
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    done: mpsc::Receiver<Result<bool, String>>,
}

impl Install {
    /// Starts installing the published music model into `dir`.
    pub fn start(ctx: egui::Context, dir: PathBuf) -> Self {
        let phase = Arc::new(AtomicU8::new(0));
        let progress = Arc::new(AtomicU32::new(f32::NAN.to_bits()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, done) = mpsc::channel();
        let (shared_phase, shared, stop) = (
            Arc::clone(&phase),
            Arc::clone(&progress),
            Arc::clone(&cancel),
        );
        let spawned = thread::Builder::new()
            .name("olio-music-model-install".to_owned())
            .spawn(move || {
                let mut shown = Instant::now();
                let result = run(&dir, &Sources::published(), Sync::Uv, |phase, done| {
                    shared_phase.store(phase.index(), Ordering::Relaxed);
                    shared.store(done.unwrap_or(f32::NAN).to_bits(), Ordering::Relaxed);
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
            eprintln!("olio: could not start installing the music model: {err}");
        }
        Self {
            phase,
            progress,
            cancel,
            done,
        }
    }

    /// The phase, and how far through it where known.
    pub fn progress(&self) -> (Phase, Option<f32>) {
        let done = f32::from_bits(self.progress.load(Ordering::Relaxed));
        (
            Phase::from_index(self.phase.load(Ordering::Relaxed)),
            done.is_finite().then_some(done),
        )
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Once finished: installed (`true`), stopped (`false`), or what went
    /// wrong.
    pub fn poll(&self) -> Option<Result<bool, String>> {
        match self.done.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the install stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;

    use sha2::{Digest, Sha256};

    use super::super::client::fake::{Fake, Settings};
    use super::super::{Layout, WEIGHTS, installed};
    use super::*;

    /// The folder GitHub's zip of the tag unpacks to.
    const SOURCE_FOLDER: &str = "ACE-Step-1.5-0.1.8";

    fn sha(data: &[u8]) -> String {
        fetch::hex(&Sha256::digest(data))
    }

    /// A zip of `files` (name, contents).
    fn zipped(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, contents) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// A gzipped tarball of `files` (name, contents).
    fn tarred(files: &[(&str, &[u8])]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for (name, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, name, *contents).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    /// A fake music model serving a uv archive, a source zip and two
    /// model files, and the sources that point at them.
    fn fixtures() -> (Fake, Sources, Vec<u8>) {
        let uv_name = if cfg!(windows) { "uv.exe" } else { "uv" };
        let uv = if cfg!(windows) {
            zipped(&[(uv_name, b"#!uv"), ("uvx.exe", b"x")])
        } else {
            tarred(&[
                ("uv-aarch64-apple-darwin/uvx", b"x"),
                (&format!("uv-aarch64-apple-darwin/{uv_name}"), b"#!uv"),
            ])
        };
        let source = zipped(&[
            (&format!("{SOURCE_FOLDER}/pyproject.toml"), b"[project]"),
            (
                &format!("{SOURCE_FOLDER}/acestep/models/turbo/__init__.py"),
                b"",
            ),
            (
                &format!("{SOURCE_FOLDER}/acestep/models/turbo/modeling.py"),
                b"# newer",
            ),
            ("../escape.txt", b"no"),
        ]);
        let weights: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let config = b"{\"model\": true}".to_vec();
        let mut files = HashMap::new();
        files.insert("uv".to_owned(), uv.clone());
        files.insert("source.zip".to_owned(), source.clone());
        files.insert("weights".to_owned(), weights.clone());
        files.insert("config".to_owned(), config.clone());
        let fake = Fake::start(Settings {
            files,
            ..Settings::default()
        });
        let at = |name: &str| format!("{}/files/{name}", fake.base);
        let download = |url: String, path: &str, data: &[u8]| Download {
            url,
            path: path.to_owned(),
            bytes: data.len() as u64,
            sha256: sha(data),
        };
        let archive = if cfg!(windows) { "uv.zip" } else { "uv.tar.gz" };
        let sources = Sources {
            uv: Some(download(at("uv"), archive, &uv)),
            source: download(at("source.zip"), "source.zip", &source),
            models: vec![
                download(at("weights"), WEIGHTS[0], &weights),
                download(at("config"), "acestep-v15-turbo/config.json", &config),
            ],
        };
        (fake, sources, weights)
    }

    #[test]
    fn a_fresh_install_lays_everything_out_and_a_second_run_only_syncs() {
        let (fake, sources, _) = fixtures();
        let dir = tempfile::tempdir().unwrap();
        let mut phases = Vec::new();
        let finished = run(dir.path(), &sources, Sync::Skip, |phase, done| {
            if phases.last() != Some(&phase) {
                phases.push(phase);
            }
            assert!(done.is_none_or(|done| (0.0..=1.0).contains(&done)));
            true
        })
        .unwrap();
        assert!(finished);
        assert_eq!(
            phases,
            [
                Phase::Uv,
                Phase::Source,
                Phase::Sync,
                Phase::Models,
                Phase::Finish
            ]
        );
        let layout = Layout::new(dir.path());
        assert_eq!(fs::read(layout.uv()).unwrap(), b"#!uv");
        assert!(layout.source().join("pyproject.toml").is_file());
        assert!(!dir.path().join("escape.txt").exists(), "stays inside");
        assert!(!layout.unpacking().exists());
        let code = layout.checkpoints().join("acestep-v15-turbo/modeling.py");
        assert_eq!(
            fs::read(code).unwrap(),
            b"# newer",
            "the model's code copied"
        );
        assert!(
            !layout
                .checkpoints()
                .join("acestep-v15-turbo/__init__.py")
                .exists()
        );
        assert!(layout.marker().is_file());
        // Only the weights count as installed: the python is not there
        // with Sync::Skip.
        assert_eq!(installed(dir.path()), None);
        fs::create_dir_all(layout.venv_python().parent().unwrap()).unwrap();
        fs::write(layout.venv_python(), "").unwrap();
        for weights in &WEIGHTS[1..] {
            let path = layout.checkpoints().join(weights);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        assert!(installed(dir.path()).is_some());

        let before = fake.requests().len();
        let mut phases = Vec::new();
        assert!(
            run(dir.path(), &sources, Sync::Skip, |phase, _| {
                phases.push(phase);
                true
            })
            .unwrap()
        );
        assert_eq!(phases, [Phase::Sync, Phase::Finish], "nothing to download");
        assert_eq!(fake.requests().len(), before);
    }

    #[test]
    fn a_stopped_download_goes_on_from_where_it_was() {
        let (fake, sources, weights) = fixtures();
        let dir = tempfile::tempdir().unwrap();
        // Stop part way through the weights.
        let mut stopped_at = None;
        let finished = run(dir.path(), &sources, Sync::Skip, |phase, done| {
            if phase == Phase::Models && done.is_some_and(|done| done > 0.3) {
                stopped_at = done;
                return false;
            }
            true
        })
        .unwrap();
        assert!(!finished);
        let layout = Layout::new(dir.path());
        let part = layout.part(&WEIGHTS[0].replace('/', "-"));
        let have = fs::metadata(&part).unwrap().len();
        assert!(
            have > 0 && have < weights.len() as u64,
            "{have} of {}",
            weights.len()
        );
        assert!(!layout.marker().exists());

        let mut lowest = 1.0f32;
        assert!(
            run(dir.path(), &sources, Sync::Skip, |phase, done| {
                if phase == Phase::Models
                    && let Some(done) = done
                {
                    lowest = lowest.min(done);
                }
                true
            })
            .unwrap()
        );
        assert!(lowest > 0.0, "went on, not over");
        let ranged = fake
            .requests()
            .into_iter()
            .find(|request| request.contains("/files/weights range=bytes="))
            .expect("asked for the rest");
        assert!(
            ranged.ends_with(&format!("range=bytes={have}-")),
            "{ranged}"
        );
        let dest = layout.checkpoints().join(WEIGHTS[0]);
        assert_eq!(fs::read(dest).unwrap(), weights, "whole and right");
        assert!(!part.exists());
    }

    #[test]
    fn a_damaged_download_is_refused_and_nothing_is_marked_installed() {
        let (_fake, mut sources, _) = fixtures();
        sources.models[1].sha256 = "0".repeat(64);
        let dir = tempfile::tempdir().unwrap();
        let err = run(dir.path(), &sources, Sync::Skip, |_, _| true).unwrap_err();
        assert!(err.contains("damaged"), "{err}");
        let layout = Layout::new(dir.path());
        assert!(!layout.marker().exists());
        assert!(
            !layout
                .checkpoints()
                .join("acestep-v15-turbo/config.json")
                .exists()
        );
        assert!(!layout.part("acestep-v15-turbo-config.json").exists());
    }

    #[test]
    fn a_far_end_without_ranges_starts_the_file_over() {
        let (fake, sources, _) = fixtures();
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(layout.part("config"), b"stale").unwrap();
        let _ = fake;
        // A source served whole despite the range: the part is replaced.
        let plain = Fake::start(Settings {
            ranges: false,
            files: HashMap::from([("config".to_owned(), b"{\"model\": true}".to_vec())]),
            ..Settings::default()
        });
        let download = Download {
            url: format!("{}/files/config", plain.base),
            path: "config".to_owned(),
            bytes: 15,
            sha256: sha(b"{\"model\": true}"),
        };
        let _ = sources;
        super::download(&download, &layout.part("config"), |_| true)
            .unwrap()
            .unwrap();
        assert_eq!(
            fs::read(layout.part("config")).unwrap(),
            b"{\"model\": true}"
        );
        assert!(plain.requests()[0].contains("range=bytes=5-"));
    }
}
