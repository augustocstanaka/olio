//! Asking the music model for a track: ACE-Step's REST API on this
//! computer. A task is released, polled until it is done, and its WAV
//! fetched. The API says only whether a task is queued, running or done,
//! never how far, so the caller shows time passing instead.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::DIT_MODEL;
use crate::fetch;

/// How often a task is asked about.
const POLL: Duration = Duration::from_millis(500);
/// Releasing a task may wait for the model to load, the first time.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);

/// What a track is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub prompt: String,
    pub seconds: u32,
    /// The same prompt and seed give the same track; `None` for a new one.
    pub seed: Option<u64>,
}

impl Request {
    /// The task as ACE-Step takes it: instrumental, a WAV, one track, the
    /// turbo model's eight steps, no language model.
    pub fn body(&self) -> Value {
        json!({
            "prompt": self.prompt,
            "lyrics": "[Instrumental]",
            "audio_duration": self.seconds,
            "audio_format": "wav",
            "batch_size": 1,
            "inference_steps": 8,
            "thinking": false,
            "model": DIT_MODEL,
            "use_random_seed": self.seed.is_none(),
            "seed": self.seed.map_or(-1, |seed| seed as i64),
        })
    }
}

/// Where a task has got to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Queued or running.
    Waiting,
    /// Done: the files to fetch, as `/v1/audio?path=…`.
    Done(Vec<String>),
    Failed(String),
}

/// A music model at a base URL, with its key when it has one.
#[derive(Clone, Debug)]
pub struct Client {
    agent: ureq::Agent,
    base: String,
    key: Option<String>,
}

impl Client {
    pub fn new(base: String, key: Option<String>) -> Self {
        Self {
            agent: fetch::agent(),
            base: base.trim_end_matches('/').to_owned(),
            key,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn bearer(&self) -> Option<String> {
        self.key.as_ref().map(|key| format!("Bearer {key}"))
    }

    /// A GET, within `timeout`.
    fn get(
        &self,
        path: &str,
        timeout: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>, String> {
        let mut request = self
            .agent
            .get(self.url(path))
            .config()
            .timeout_global(Some(timeout))
            .build();
        if let Some(bearer) = self.bearer() {
            request = request.header("Authorization", bearer);
        }
        request.call().map_err(|err| err.to_string())
    }

    /// A POST of JSON, within `timeout`, giving the `data` of the reply.
    fn post(&self, path: &str, body: &Value, timeout: Duration) -> Result<Value, String> {
        let mut request = self
            .agent
            .post(self.url(path))
            .config()
            .timeout_global(Some(timeout))
            .build()
            .header("Content-Type", "application/json");
        if let Some(bearer) = self.bearer() {
            request = request.header("Authorization", bearer);
        }
        let mut response = request
            .send(body.to_string().as_bytes())
            .map_err(|err| err.to_string())?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|err| err.to_string())?;
        let reply: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let detail = reply
                .get("detail")
                .or_else(|| reply.get("error"))
                .and_then(Value::as_str)
                .unwrap_or(text.trim());
            return Err(format!("the music model answered {status}: {detail}"));
        }
        if let Some(error) = reply.get("error").and_then(Value::as_str) {
            return Err(format!("the music model: {error}"));
        }
        Ok(reply.get("data").cloned().unwrap_or(Value::Null))
    }

    /// True once the music model answers.
    pub fn healthy(&self) -> bool {
        self.get("/health", HEALTH_TIMEOUT)
            .is_ok_and(|response| response.status().is_success())
    }

    /// Waits until the music model answers: `Ok(false)` if `cancel` was
    /// set first, an error once `deadline` has passed.
    pub fn wait_healthy(&self, deadline: Duration, cancel: &AtomicBool) -> Result<bool, String> {
        let started = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            if self.healthy() {
                return Ok(true);
            }
            if started.elapsed() > deadline {
                return Err("the music model did not start".to_owned());
            }
            if !pause(POLL, cancel) {
                return Ok(false);
            }
        }
    }

    /// Releases the task, giving its id.
    pub fn release(&self, request: &Request) -> Result<String, String> {
        let data = self.post("/release_task", &request.body(), RELEASE_TIMEOUT)?;
        data.get("task_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "the music model gave the task no id".to_owned())
    }

    /// Where the task has got to.
    pub fn state(&self, task_id: &str) -> Result<State, String> {
        let data = self.post(
            "/query_result",
            &json!({ "task_id_list": [task_id] }),
            QUERY_TIMEOUT,
        )?;
        let item = data
            .as_array()
            .and_then(|items| items.iter().find(|item| item["task_id"] == task_id))
            .ok_or_else(|| "the music model lost the task".to_owned())?;
        // `result` is JSON in a string.
        let result: Vec<Value> = item
            .get("result")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();
        match item.get("status").and_then(Value::as_i64) {
            Some(0) => Ok(State::Waiting),
            Some(1) => {
                let files: Vec<String> = result
                    .iter()
                    .filter_map(|entry| entry.get("file").and_then(Value::as_str))
                    .filter(|file| !file.is_empty())
                    .map(str::to_owned)
                    .collect();
                if files.is_empty() {
                    Ok(State::Failed("the music model made no file".to_owned()))
                } else {
                    Ok(State::Done(files))
                }
            }
            _ => {
                let why = result
                    .iter()
                    .find_map(|entry| {
                        entry
                            .get("status_message")
                            .or_else(|| entry.get("error"))
                            .and_then(Value::as_str)
                    })
                    .filter(|why| !why.is_empty())
                    .unwrap_or("the music model could not make the track");
                Ok(State::Failed(why.to_owned()))
            }
        }
    }

    /// Fetches a made file (`/v1/audio?path=…`, or a whole URL) into `into`.
    pub fn fetch(&self, file: &str, into: &Path) -> Result<(), String> {
        let path = file
            .strip_prefix(self.base.as_str())
            .filter(|_| file.starts_with("http"))
            .unwrap_or(file);
        if path.starts_with("http") {
            return Err("the music model put the track somewhere else".to_owned());
        }
        let response = self.get(path, RELEASE_TIMEOUT)?;
        if !response.status().is_success() {
            return Err(format!(
                "the music model answered {} for the track",
                response.status().as_u16()
            ));
        }
        let mut body = response.into_body().into_reader();
        let mut out = fs::File::create(into).map_err(|err| format!("{}: {err}", into.display()))?;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let read = body.read(&mut buffer).map_err(|err| err.to_string())?;
            if read == 0 {
                return Ok(());
            }
            out.write_all(&buffer[..read])
                .map_err(|err| format!("{}: {err}", into.display()))?;
        }
    }

    /// The whole exchange: releases the task, waits for it, fetches the
    /// track into `into`. `None` if `cancel` was set first.
    pub fn generate(
        &self,
        request: &Request,
        into: &Path,
        cancel: &AtomicBool,
    ) -> Result<Option<()>, String> {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let task_id = self.release(request)?;
        let started = Instant::now();
        loop {
            if !pause(POLL, cancel) {
                return Ok(None);
            }
            match self.state(&task_id) {
                Ok(State::Waiting) => {}
                Ok(State::Failed(why)) => return Err(why),
                Ok(State::Done(files)) => {
                    let file = files.first().ok_or("the music model made no file")?;
                    self.fetch(file, into)?;
                    return Ok(Some(()));
                }
                // Loading the model for its first task, the server answers
                // nothing for minutes: not yet a failure.
                Err(err) if err.contains("timeout") && started.elapsed() < RELEASE_TIMEOUT => {}
                Err(err) => return Err(err),
            }
        }
    }
}

/// Sleeps `for_`, waking early if `cancel` is set: true if it was not.
fn pause(for_: Duration, cancel: &AtomicBool) -> bool {
    let until = Instant::now() + for_;
    while Instant::now() < until {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
    !cancel.load(Ordering::Relaxed)
}

/// A music model that answers like ACE-Step, for tests: it makes a WAV
/// after a few polls, or fails, and serves files with ranges.
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use serde_json::{Value, json};

    /// How the fake behaves.
    #[derive(Clone)]
    pub(crate) struct Settings {
        /// How many polls a task takes.
        pub polls: usize,
        /// Every task fails.
        pub fail: bool,
        /// The WAV a task makes.
        pub audio: Vec<u8>,
        /// Files served under `/files/<name>`.
        pub files: HashMap<String, Vec<u8>>,
        /// Whether `Range` is honoured (else the whole file comes back).
        pub ranges: bool,
        /// The key `Authorization: Bearer` must carry, if any.
        pub key: Option<String>,
        /// How many health checks fail before one succeeds.
        pub unhealthy: usize,
    }

    impl Default for Settings {
        fn default() -> Self {
            Self {
                polls: 2,
                fail: false,
                audio: Vec::new(),
                files: HashMap::new(),
                ranges: true,
                key: None,
                unhealthy: 0,
            }
        }
    }

    pub(crate) struct Fake {
        pub base: String,
        requests: Arc<Mutex<Vec<String>>>,
        bodies: Arc<Mutex<Vec<Value>>>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Fake {
        pub(crate) fn start(settings: Settings) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let bodies = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let state = Arc::new(State {
                settings,
                polls: AtomicUsize::new(0),
                health: AtomicUsize::new(0),
                requests: Arc::clone(&requests),
                bodies: Arc::clone(&bodies),
            });
            let stopping = Arc::clone(&stop);
            let thread = thread::spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let state = Arc::clone(&state);
                            thread::spawn(move || serve(stream, &state));
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                base,
                requests,
                bodies,
                stop,
                thread: Some(thread),
            }
        }

        /// "POST /release_task", "GET /files/a range=bytes=3-"…
        pub(crate) fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        /// The tasks released.
        pub(crate) fn bodies(&self) -> Vec<Value> {
            self.bodies.lock().unwrap().clone()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    struct State {
        settings: Settings,
        polls: AtomicUsize,
        health: AtomicUsize,
        requests: Arc<Mutex<Vec<String>>>,
        bodies: Arc<Mutex<Vec<Value>>>,
    }

    fn serve(mut stream: TcpStream, state: &State) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => return,
            }
        }
        let head = String::from_utf8_lossy(&head).into_owned();
        let mut lines = head.lines();
        let request_line = lines.next().unwrap_or_default();
        let mut parts = request_line.split(' ');
        let method = parts.next().unwrap_or_default().to_owned();
        let target = parts.next().unwrap_or_default().to_owned();
        let mut headers = HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
            }
        }
        let length: usize = headers
            .get("content-length")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; length];
        if length > 0 && stream.read_exact(&mut body).is_err() {
            return;
        }
        let range = headers.get("range").cloned();
        state.requests.lock().unwrap().push(match &range {
            Some(range) => format!("{method} {target} range={range}"),
            None => format!("{method} {target}"),
        });
        let authorised = match &state.settings.key {
            None => true,
            Some(key) => headers.get("authorization") == Some(&format!("Bearer {key}")),
        };
        let path = target.split('?').next().unwrap_or_default();
        let (status, content_type, payload, extra) = if !authorised {
            (
                401,
                "application/json",
                br#"{"detail":"Invalid API key"}"#.to_vec(),
                None,
            )
        } else {
            match (method.as_str(), path) {
                ("GET", "/health") => {
                    let checks = state.health.fetch_add(1, Ordering::Relaxed);
                    if checks < state.settings.unhealthy {
                        (503, "text/plain", b"starting".to_vec(), None)
                    } else {
                        (
                            200,
                            "application/json",
                            br#"{"status":"ok"}"#.to_vec(),
                            None,
                        )
                    }
                }
                ("POST", "/release_task") => {
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    state.bodies.lock().unwrap().push(body);
                    state.polls.store(0, Ordering::Relaxed);
                    let reply = wrap(json!({
                        "task_id": "task-1", "status": "queued", "queue_position": 0
                    }));
                    (
                        200,
                        "application/json",
                        reply.to_string().into_bytes(),
                        None,
                    )
                }
                ("POST", "/query_result") => {
                    let polls = state.polls.fetch_add(1, Ordering::Relaxed) + 1;
                    let item = if state.settings.fail {
                        json!({ "task_id": "task-1", "status": 2,
                                "result": json!([{ "file": "", "status": 2,
                                    "status_message": "out of memory" }]).to_string() })
                    } else if polls < state.settings.polls {
                        json!({ "task_id": "task-1", "status": 0, "result": "[]" })
                    } else {
                        json!({ "task_id": "task-1", "status": 1,
                                "result": json!([{ "file": "/v1/audio?path=%2Ftmp%2Fout.wav",
                                    "status": 1 }]).to_string() })
                    };
                    let reply = wrap(json!([item]));
                    (
                        200,
                        "application/json",
                        reply.to_string().into_bytes(),
                        None,
                    )
                }
                ("GET", "/v1/audio") => (200, "audio/wav", state.settings.audio.clone(), None),
                ("GET", file) if file.starts_with("/files/") => {
                    match state.settings.files.get(&file["/files/".len()..]) {
                        None => (404, "text/plain", b"no such file".to_vec(), None),
                        Some(data) => match range.as_deref().filter(|_| state.settings.ranges) {
                            Some(range) => {
                                let from: usize = range
                                    .trim_start_matches("bytes=")
                                    .trim_end_matches('-')
                                    .parse()
                                    .unwrap_or(0);
                                if from >= data.len() {
                                    (416, "text/plain", Vec::new(), None)
                                } else {
                                    let extra = format!(
                                        "Content-Range: bytes {from}-{}/{}\r\n",
                                        data.len() - 1,
                                        data.len()
                                    );
                                    (
                                        206,
                                        "application/octet-stream",
                                        data[from..].to_vec(),
                                        Some(extra),
                                    )
                                }
                            }
                            None => (200, "application/octet-stream", data.clone(), None),
                        },
                    }
                }
                _ => (404, "text/plain", b"no such route".to_vec(), None),
            }
        };
        let reason = match status {
            200 => "OK",
            206 => "Partial Content",
            401 => "Unauthorized",
            404 => "Not Found",
            416 => "Range Not Satisfiable",
            _ => "Service Unavailable",
        };
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
            payload.len(),
            extra.unwrap_or_default()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&payload);
        let _ = stream.flush();
    }

    fn wrap(data: Value) -> Value {
        json!({ "data": data, "code": 200, "error": Value::Null, "timestamp": 0, "extra": Value::Null })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::fake::{Fake, Settings};
    use super::*;
    use olio_audio::{Clip, WavFormat};

    /// A second of stereo 48 kHz, as ACE-Step makes it.
    pub(crate) fn wav() -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        let samples: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let s = (i as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU).sin() * 0.3;
                [s, s]
            })
            .collect();
        Clip::new(48_000, 2, samples)
            .write_wav(&path, WavFormat::Int16)
            .unwrap();
        fs::read(path).unwrap()
    }

    #[test]
    fn a_prompt_becomes_a_wav_through_release_poll_and_fetch() {
        let fake = Fake::start(Settings {
            audio: wav(),
            key: Some("secret".to_owned()),
            ..Settings::default()
        });
        let client = Client::new(fake.base.clone(), Some("secret".to_owned()));
        assert!(client.healthy());
        let request = Request {
            prompt: "warm piano over soft rain".to_owned(),
            seconds: 30,
            seed: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("track.wav");
        let cancel = AtomicBool::new(false);
        assert_eq!(client.generate(&request, &into, &cancel), Ok(Some(())));
        let clip = olio_audio::decode_file(&into).unwrap();
        assert_eq!((clip.rate(), clip.channels()), (48_000, 2));

        let body = &fake.bodies()[0];
        assert_eq!(body["lyrics"], "[Instrumental]");
        assert_eq!(body["audio_duration"], 30);
        assert_eq!(body["audio_format"], "wav");
        assert_eq!(body["batch_size"], 1);
        assert_eq!(body["thinking"], false);
        assert_eq!(body["use_random_seed"], true);
        assert_eq!(body["model"], DIT_MODEL);
        let requests = fake.requests();
        assert_eq!(requests[0], "GET /health");
        assert_eq!(requests[1], "POST /release_task");
        assert_eq!(requests[2], "POST /query_result");
        assert_eq!(requests[3], "POST /query_result");
        assert_eq!(requests[4], "GET /v1/audio?path=%2Ftmp%2Fout.wav");

        // A seed asked for is sent.
        let seeded = Request {
            seed: Some(7),
            ..request
        };
        assert_eq!(seeded.body()["seed"], 7);
        assert_eq!(seeded.body()["use_random_seed"], false);
    }

    #[test]
    fn the_wrong_key_a_failed_task_and_a_cancel_are_told_apart() {
        let fake = Fake::start(Settings {
            fail: true,
            key: Some("secret".to_owned()),
            ..Settings::default()
        });
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("track.wav");
        let cancel = AtomicBool::new(false);
        let request = Request {
            prompt: "x".to_owned(),
            seconds: 30,
            seed: None,
        };

        let wrong = Client::new(fake.base.clone(), Some("nope".to_owned()));
        let err = wrong.generate(&request, &into, &cancel).unwrap_err();
        assert!(err.contains("401"), "{err}");

        let client = Client::new(fake.base.clone(), Some("secret".to_owned()));
        assert_eq!(
            client.generate(&request, &into, &cancel),
            Err("out of memory".to_owned())
        );

        cancel.store(true, Ordering::Relaxed);
        assert_eq!(client.generate(&request, &into, &cancel), Ok(None));
        assert!(!into.exists());
    }

    #[test]
    fn waiting_for_health_gives_up_on_cancel_or_a_deadline() {
        let fake = Fake::start(Settings {
            unhealthy: 2,
            ..Settings::default()
        });
        let client = Client::new(fake.base.clone(), None);
        let cancel = AtomicBool::new(false);
        assert_eq!(
            client.wait_healthy(Duration::from_secs(10), &cancel),
            Ok(true)
        );
        assert_eq!(fake.requests().len(), 3, "two refusals, then ok");

        let nobody = Client::new("http://127.0.0.1:9".to_owned(), None);
        assert!(!nobody.healthy());
        assert!(
            nobody
                .wait_healthy(Duration::from_millis(600), &cancel)
                .is_err()
        );
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            nobody.wait_healthy(Duration::from_secs(10), &cancel),
            Ok(false)
        );
    }
}
