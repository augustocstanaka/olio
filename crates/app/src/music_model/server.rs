//! The music model running: ACE-Step's API server as a child process of
//! the app, on 127.0.0.1 at a port of its own with a key of its own,
//! started for a prompted track and stopped when it has been idle a
//! while (it holds about 6 GB of memory) or when the app quits.

use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::client::Client;
use super::{DIT_MODEL, Layout, lm_backend};
use crate::fetch;

/// How long the music model stays up unused.
pub const IDLE_STOP: Duration = Duration::from_secs(10 * 60);
/// How long it may take to answer `/health` after starting.
pub const START_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// ACE-Step's API server, run from the environment uv made.
const ENTRY: &str = "import sys\nfrom acestep.api_server import main\nsys.exit(main())";

pub struct Server {
    child: Option<Child>,
    base: String,
    key: Option<String>,
    last_used: Instant,
}

impl Server {
    /// Starts the music model installed in `dir`. It answers a little
    /// later: see [`Client::wait_healthy`].
    pub fn start(dir: &Path) -> Result<Self, String> {
        let layout = Layout::new(dir);
        let python = layout.venv_python();
        if !python.is_file() {
            return Err("the music model is not installed".to_owned());
        }
        let port = free_port()?;
        let key = random_key();
        let log_path = layout.server_log();
        let log =
            fs::File::create(&log_path).map_err(|err| format!("{}: {err}", log_path.display()))?;
        let errors = log.try_clone().map_err(|err| err.to_string())?;
        let mut command = Command::new(&python);
        command
            .arg("-c")
            .arg(ENTRY)
            .args(["--host", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .args(["--api-key", &key])
            .current_dir(layout.source())
            .env("VIRTUAL_ENV", layout.venv())
            .env("ACESTEP_CONFIG_PATH", DIT_MODEL)
            .env("ACESTEP_INIT_LLM", "false")
            .env("ACESTEP_NO_INIT", "true")
            .env("ACESTEP_LM_BACKEND", lm_backend())
            .env("ACESTEP_DOWNLOAD_SOURCE", "huggingface")
            .env("HF_HUB_OFFLINE", "1")
            .env("TOKENIZERS_PARALLELISM", "false")
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(errors));
        Self::spawn_with(command, format!("http://127.0.0.1:{port}"), Some(key))
    }

    /// Runs `command` as the music model answering at `base`.
    pub fn spawn_with(
        mut command: Command,
        base: String,
        key: Option<String>,
    ) -> Result<Self, String> {
        super::install::quiet(&mut command);
        let child = command
            .spawn()
            .map_err(|err| format!("could not start the music model: {err}"))?;
        Ok(Self {
            child: Some(child),
            base,
            key,
            last_used: Instant::now(),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn client(&self) -> Client {
        Client::new(self.base.clone(), self.key.clone())
    }

    /// Note that it was used, so it is not stopped for idling.
    pub fn touch(&mut self) {
        self.last_used = Instant::now();
    }

    /// How long since it was used.
    pub fn idle(&self) -> Duration {
        self.last_used.elapsed()
    }

    /// True while the process runs.
    pub fn alive(&mut self) -> bool {
        match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Stops the process, if it still runs.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A port nothing listens on right now.
fn free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|err| format!("no free port: {err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("no free port: {err}"))?
        .port();
    Ok(port)
}

/// 32 hex characters nobody else knows: from the clock, the process and
/// a fresh allocation's address, mixed.
fn random_key() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let salt = Box::new(0u8);
    let address = &*salt as *const u8 as u64;
    let mut state = nanos ^ (u64::from(std::process::id()) << 32) ^ address;
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        // xorshift64*
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let out = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        chunk.copy_from_slice(&out.to_le_bytes());
    }
    fetch::hex(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_long_and_differ() {
        let a = random_key();
        let b = random_key();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(free_port().unwrap() > 0);
    }

    #[cfg(unix)]
    #[test]
    fn stopping_or_dropping_the_server_ends_its_process() {
        let mut command = Command::new("sleep");
        command.arg("60");
        let mut server =
            Server::spawn_with(command, "http://127.0.0.1:1".to_owned(), None).unwrap();
        let pid = server.child.as_ref().unwrap().id();
        assert!(server.alive());
        assert!(server.idle() < Duration::from_secs(1));
        server.touch();
        server.stop();
        assert!(!server.alive());
        let gone = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .unwrap();
        assert!(!gone.status.success(), "no process {pid} any more");

        let mut command = Command::new("sleep");
        command.arg("60");
        let server = Server::spawn_with(command, "http://127.0.0.1:1".to_owned(), None).unwrap();
        let pid = server.child.as_ref().unwrap().id();
        assert_eq!(server.client().base(), "http://127.0.0.1:1");
        drop(server);
        let gone = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .unwrap();
        assert!(!gone.status.success(), "dropped: no process {pid} any more");

        let missing = Server::start(Path::new("/nowhere/at/all"));
        assert_eq!(
            missing.err(),
            Some("the music model is not installed".to_owned())
        );
    }
}
