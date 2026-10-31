//! Downloading a file over HTTPS, with progress and a stop: the instrument
//! pack, and the music model's archives and model files, which are big
//! enough to go on from where a stopped download left off.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

/// How long to wait for the far end to answer at all.
const CONNECT: Duration = Duration::from_secs(20);

/// An HTTP client that reports every status, so a `206` or a `416` can be
/// read, and never gives up on a slow body.
pub fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .build();
    ureq::Agent::new_with_config(config)
}

/// What a finished download gives: how many bytes the file has now, and
/// their SHA-256 (of the whole file, however much of it was downloaded
/// earlier).
pub struct Fetched {
    pub bytes: u64,
    pub sha256: String,
}

/// Downloads `url` into `path`, telling `progress` how far, 0 to 1, against
/// `total` bytes (or the far end's content length, or what has arrived);
/// stops with `None` when `progress` says to. With `resume`, a file already
/// at `path` is kept and the rest asked for; a far end that sends the whole
/// file instead starts it over.
pub fn download(
    url: &str,
    path: &Path,
    resume: bool,
    total: Option<u64>,
    mut progress: impl FnMut(f32) -> bool,
) -> Result<Option<Fetched>, String> {
    let have = if resume {
        fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
    } else {
        0
    };
    let mut request = agent().get(url);
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }
    let response = request.call().map_err(|err| err.to_string())?;
    let status = response.status().as_u16();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let (mut file, mut done) = match status {
        206 if have > 0 => {
            let file = fs::OpenOptions::new()
                .append(true)
                .open(path)
                .map_err(|err| format!("{}: {err}", path.display()))?;
            (file, have)
        }
        200 => {
            let file =
                fs::File::create(path).map_err(|err| format!("{}: {err}", path.display()))?;
            (file, 0)
        }
        416 if have > 0 => {
            // Nothing left to ask for: the file was already complete.
            return finished(path);
        }
        status => return Err(format!("{url}: HTTP {status}")),
    };
    let total = total.or_else(|| {
        header("content-length")
            .and_then(|value| value.parse::<u64>().ok())
            .map(|length| length + done)
    });
    let mut body = response.into_body().into_reader();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = body.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("{}: {err}", path.display()))?;
        done += read as u64;
        let fraction = match total {
            Some(total) => (done as f32 / total.max(1) as f32).min(1.0),
            None => 0.0,
        };
        if !progress(fraction) {
            return Ok(None);
        }
    }
    drop(file);
    finished(path)
}

/// The file's size and SHA-256.
fn finished(path: &Path) -> Result<Option<Fetched>, String> {
    let mut file = fs::File::open(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("{}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok(Some(Fetched {
        bytes,
        sha256: hex(&hasher.finalize()),
    }))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// "71 MB".
pub fn megabytes(bytes: u64) -> String {
    format!("{} MB", (bytes as f64 / 1e6).round())
}

/// "about 10 GB".
pub fn about_gigabytes(bytes: u64) -> String {
    format!("about {} GB", (bytes as f64 / 1e9).round())
}
