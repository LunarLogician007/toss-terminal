//! Built-in speech-to-text (TOSS Terminal): Cactus Compute's Whistle on their
//! Needle engine, on the CPU, with the 16.9 MB model downloaded once into the
//! app's data folder. Audio never leaves the machine; the only network use is
//! fetching the model file itself.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct ModelSpec {
    pub id: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

/// The model on offer, from Cactus-Compute/whistle on Hugging Face
/// (Apache-2.0), pinned to a revision.
pub const MODELS: &[ModelSpec] = &[ModelSpec {
    id: "whistle",
    file: "whistle.cact",
    url: "https://huggingface.co/Cactus-Compute/whistle/resolve/b358ddadd89b7a713b5aa131f23032d3cca1b251/whistle.cact",
    bytes: 16_919_407,
    sha256: "b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb",
}];

/// The Whisper models earlier builds downloaded, deleted on startup.
const LEGACY_FILES: &[&str] = &["ggml-tiny.en-q5_1.bin", "ggml-base.en-q5_1.bin"];

const SAMPLE_RATE: usize = 16_000;
/// Whistle reads at most 30 s per pass.
#[cfg_attr(not(needle), allow(dead_code))]
const MAX_CHUNK_SAMPLES: usize = 30 * SAMPLE_RATE;
/// A gap between words this long ends a phrase.
const PHRASE_PAUSE_MS: i64 = 200;
/// Room for a full transcript (320 tokens) with per-word times.
#[cfg_attr(not(needle), allow(dead_code))]
const OUT_CAPACITY: usize = 64 * 1024;
#[cfg_attr(not(needle), allow(dead_code))]
const LANGUAGE: &str = "en";
/// Terminal words Whistle would otherwise hear as English ("git" as "deep").
#[cfg_attr(not(needle), allow(dead_code))]
const KEYWORDS: &[&str] = &[
    "git", "pnpm", "npm", "npx", "yarn", "cd", "ls", "sudo", "cargo", "docker", "kubectl", "ssh",
    "grep", "vim", "tmux", "brew", "curl", "mkdir", "chmod",
];
pub const DOWNLOAD_EVENT: &str = "stt://download";
const MODEL_HEADER: &str = "x-stt-model";
const UNSUPPORTED: &str = "Built-in dictation isn't available on this platform yet.";
/// The system's curl does the one download this module needs, so the app
/// doesn't carry an HTTP and TLS stack for it (macOS always ships curl).
#[cfg(target_os = "macos")]
const CURL: &str = "/usr/bin/curl";
#[cfg(target_os = "windows")]
const CURL: &str = "curl.exe";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const CURL: &str = "curl";

pub fn model_spec(id: &str) -> Result<&'static ModelSpec, String> {
    MODELS
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("Unknown speech model: {id}"))
}

pub fn model_path(dir: &Path, spec: &ModelSpec) -> PathBuf {
    dir.join(spec.file)
}

fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("models"))
        .map_err(|e| e.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn check_digest(actual_hex: &str, spec: &ModelSpec) -> Result<(), String> {
    if actual_hex.eq_ignore_ascii_case(spec.sha256) {
        Ok(())
    } else {
        Err("the speech model's checksum doesn't match".into())
    }
}

/// The file at `path` has the model's exact size and SHA-256.
pub fn verify_file(path: &Path, spec: &ModelSpec) -> Result<(), String> {
    let len = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len != spec.bytes {
        return Err(format!("expected {} bytes, found {len}", spec.bytes));
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    check_digest(&hex(&hasher.finalize()), spec)
}

/// Delete the Whisper models earlier builds left in `dir`.
pub fn remove_legacy(dir: &Path) {
    for name in LEGACY_FILES {
        let _ = fs::remove_file(dir.join(name));
        let _ = fs::remove_file(dir.join(format!("{name}.part")));
    }
}

/// Free the disk space the Whisper models took (60 MB or more).
pub fn remove_legacy_models(app: &AppHandle) {
    if let Ok(dir) = models_dir(app) {
        remove_legacy(&dir);
    }
}

/// Little-endian f32 samples, as a `Float32Array`'s bytes arrive.
pub fn samples_from_le_bytes(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() % 4 != 0 {
        return Err("the audio isn't whole 32-bit samples".into());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Consecutive sample ranges of at most `max` covering `len` samples.
pub fn chunk_ranges(len: usize, max: usize) -> Vec<(usize, usize)> {
    (0..len)
        .step_by(max.max(1))
        .map(|start| (start, (start + max).min(len)))
        .collect()
}

fn ms(seconds: f64) -> i64 {
    (seconds * 1000.0).round() as i64
}

/// One phrase, times in ms from the start of the audio given.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    text: String,
    start_ms: i64,
    end_ms: i64,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Word {
    word: String,
    start: f64,
    end: f64,
}

/// What `needle_transcribe` writes, the fields this module reads.
#[derive(Debug, PartialEq, Deserialize)]
pub struct Transcript {
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<Word>,
}

/// The engine's NUL-terminated JSON output.
pub fn parse_output(out: &[u8]) -> Result<Transcript, String> {
    let end = out
        .iter()
        .position(|&b| b == 0)
        .ok_or("the transcript didn't fit")?;
    serde_json::from_slice(&out[..end]).map_err(|e| format!("unreadable transcript: {e}"))
}

fn ends_clause(word: &str) -> bool {
    word.ends_with(['.', ',', '?', '!', ';', ':'])
}

/// Group a pass's words into phrases at punctuation and pauses, so live
/// dictation trims typed audio where you paused, not mid-sentence.
/// `offset_ms` places a chunk's times within the whole clip.
pub fn phrases(transcript: &Transcript, offset_ms: i64, chunk_ms: i64) -> Vec<Segment> {
    let words = &transcript.words;
    if words.is_empty() {
        let text = transcript.text.trim();
        if text.is_empty() {
            return Vec::new();
        }
        return vec![Segment {
            text: text.to_string(),
            start_ms: offset_ms,
            end_ms: offset_ms + chunk_ms,
        }];
    }
    let mut out = Vec::new();
    let mut first = 0;
    for i in 0..words.len() {
        let last = i + 1 == words.len();
        let pause = !last && ms(words[i + 1].start) - ms(words[i].end) >= PHRASE_PAUSE_MS;
        if last || pause || ends_clause(&words[i].word) {
            let text = words[first..=i]
                .iter()
                .map(|w| w.word.trim())
                .collect::<Vec<_>>()
                .join(" ");
            out.push(Segment {
                text,
                start_ms: offset_ms + ms(words[first].start),
                end_ms: offset_ms + ms(words[i].end),
            });
            first = i + 1;
        }
    }
    out
}

/// The Needle engine's C API (needle.h), linked by build.rs.
#[cfg(needle)]
mod ffi {
    use std::ffi::{c_char, c_int, c_uchar, c_ulonglong};

    extern "C" {
        pub fn needle_load(cact: *const c_uchar, n: c_ulonglong) -> c_int;
        pub fn needle_last_error() -> *const c_char;
        pub fn needle_transcribe(
            pcm: *const f32,
            samples: c_int,
            language: *const c_char,
            keywords: *const c_char,
            word_timestamps: c_int,
            out: *mut c_char,
            out_capacity: c_int,
        ) -> c_int;
    }
}

/// The engine is process-global and not thread-safe: every call holds this
/// lock, which also records the model loaded into it.
#[cfg(needle)]
static ENGINE: Mutex<Option<&'static str>> = Mutex::new(None);

#[cfg(needle)]
mod engine {
    use std::ffi::{CStr, CString};

    use super::*;

    fn last_error() -> String {
        // SAFETY: the engine returns null or a NUL-terminated string it owns,
        // valid until the next call; it is copied out under the lock.
        let ptr = unsafe { ffi::needle_last_error() };
        if ptr.is_null() {
            return "unknown error".into();
        }
        let msg = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
        if msg.is_empty() {
            "unknown error".into()
        } else {
            msg
        }
    }

    /// Load `spec` into the engine unless it is there already. The engine
    /// copies the model, so the file's bytes are dropped afterwards.
    pub fn ensure_loaded(
        loaded: &mut Option<&'static str>,
        spec: &'static ModelSpec,
        path: &Path,
    ) -> Result<(), String> {
        if *loaded == Some(spec.id) {
            return Ok(());
        }
        let bytes = fs::read(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "The speech model isn't downloaded.".to_string(),
            _ => format!("Couldn't read the speech model: {e}"),
        })?;
        // SAFETY: a valid buffer and its length; the engine copies it.
        let rc = unsafe { ffi::needle_load(bytes.as_ptr(), bytes.len() as u64) };
        if rc < 0 {
            return Err(format!("Couldn't load the speech model: {}", last_error()));
        }
        *loaded = Some(spec.id);
        Ok(())
    }

    pub fn load(spec: &'static ModelSpec, path: &Path) -> Result<(), String> {
        let mut guard = ENGINE
            .lock()
            .map_err(|_| "the speech model is unavailable".to_string())?;
        ensure_loaded(&mut guard, spec, path)
    }

    /// Phrases with times; clips over 30 s go in 30 s chunks.
    pub fn transcribe(
        spec: &'static ModelSpec,
        path: &Path,
        samples: &[f32],
    ) -> Result<Vec<Segment>, String> {
        let mut guard = ENGINE
            .lock()
            .map_err(|_| "the speech model is unavailable".to_string())?;
        ensure_loaded(&mut guard, spec, path)?;
        let language = CString::new(LANGUAGE).map_err(|e| e.to_string())?;
        let keywords = CString::new(KEYWORDS.join("\n")).map_err(|e| e.to_string())?;
        let mut out = vec![0u8; OUT_CAPACITY];
        let mut segments = Vec::new();
        for (start, end) in chunk_ranges(samples.len(), MAX_CHUNK_SAMPLES) {
            let chunk = &samples[start..end];
            out.fill(0);
            // SAFETY: the chunk and the output buffer are valid for the
            // lengths given, and the strings are NUL-terminated. The lock
            // keeps the engine single-threaded.
            let rc = unsafe {
                ffi::needle_transcribe(
                    chunk.as_ptr(),
                    chunk.len() as i32,
                    language.as_ptr(),
                    keywords.as_ptr(),
                    1,
                    out.as_mut_ptr().cast(),
                    OUT_CAPACITY as i32,
                )
            };
            if rc < 0 {
                return Err(format!("Transcription failed: {}", last_error()));
            }
            let transcript = parse_output(&out)?;
            let offset_ms = (start * 1000 / SAMPLE_RATE) as i64;
            let chunk_ms = (chunk.len() * 1000 / SAMPLE_RATE) as i64;
            segments.extend(phrases(&transcript, offset_ms, chunk_ms));
        }
        Ok(segments)
    }
}

#[cfg(not(needle))]
mod engine {
    use super::*;

    pub fn load(_spec: &'static ModelSpec, _path: &Path) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn transcribe(
        _spec: &'static ModelSpec,
        _path: &Path,
        _samples: &[f32],
    ) -> Result<Vec<Segment>, String> {
        Err(UNSUPPORTED.into())
    }
}

fn supported() -> Result<(), String> {
    if cfg!(needle) {
        Ok(())
    } else {
        Err(UNSUPPORTED.into())
    }
}

#[derive(Default)]
pub struct SttState {
    /// Models being downloaded now. The settings window and the main window
    /// can both ask; only one may write the file.
    downloading: Arc<Mutex<HashSet<&'static str>>>,
}

/// Marks a model as downloading until dropped.
struct DownloadGuard {
    set: Arc<Mutex<HashSet<&'static str>>>,
    id: &'static str,
}

impl DownloadGuard {
    fn claim(set: &Arc<Mutex<HashSet<&'static str>>>, id: &'static str) -> Option<Self> {
        let mut held = set.lock().ok()?;
        if !held.insert(id) {
            return None;
        }
        Some(Self {
            set: set.clone(),
            id,
        })
    }
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        if let Ok(mut held) = self.set.lock() {
            held.remove(self.id);
        }
    }
}

#[derive(Serialize)]
pub struct ModelStatus {
    ready: bool,
    bytes: u64,
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    model: &'static str,
    received: u64,
    total: u64,
}

/// Whether the model is on disk at full size (checked by hash when downloaded).
#[tauri::command]
pub fn stt_model_status(app: AppHandle, model: String) -> Result<ModelStatus, String> {
    supported()?;
    let spec = model_spec(&model)?;
    let path = model_path(&models_dir(&app)?, spec);
    let ready = fs::metadata(&path)
        .map(|m| m.len() == spec.bytes)
        .unwrap_or(false);
    Ok(ModelStatus {
        ready,
        bytes: spec.bytes,
    })
}

/// Fetch the model into a `.part` file and move it into place only once its
/// size and checksum match.
#[tauri::command]
pub async fn stt_download_model(
    app: AppHandle,
    state: State<'_, SttState>,
    model: String,
) -> Result<(), String> {
    supported()?;
    let spec = model_spec(&model)?;
    let Some(_guard) = DownloadGuard::claim(&state.downloading, spec.id) else {
        return Err("it's already downloading".into());
    };
    let dir = models_dir(&app)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = model_path(&dir, spec);
    let part = dir.join(format!("{}.part", spec.file));
    if let Err(e) = download_to(&app, spec, &part).await {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    fs::rename(&part, &dest).map_err(|e| e.to_string())
}

/// Download progress in whole percent, never past 100.
pub fn progress_pct(received: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    (received.saturating_mul(100) / total).min(100)
}

async fn download_to(app: &AppHandle, spec: &'static ModelSpec, part: &Path) -> Result<(), String> {
    let (app, part) = (app.clone(), part.to_path_buf());
    tauri::async_runtime::spawn_blocking(move || curl_download(&app, spec, &part))
        .await
        .map_err(|e| e.to_string())?
}

/// Fetch with curl (HTTPS only, redirects included, capped at the model's
/// size), report progress from the file's growth, then check size and SHA-256.
fn curl_download(app: &AppHandle, spec: &'static ModelSpec, part: &Path) -> Result<(), String> {
    let mut child = Command::new(CURL)
        .args(["--fail", "--location", "--silent", "--show-error"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--max-filesize", &spec.bytes.to_string()])
        .arg("--output")
        .arg(part)
        .arg(spec.url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start curl: {e}"))?;
    let emit = |received: u64| {
        let _ = app.emit(
            DOWNLOAD_EVENT,
            DownloadProgress {
                model: spec.id,
                received: received.min(spec.bytes),
                total: spec.bytes,
            },
        );
    };
    let mut last_pct = u64::MAX;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                let mut err = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut err);
                }
                return Err(format!("the download failed: {}", err.trim()));
            }
            break;
        }
        let received = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        let pct = progress_pct(received, spec.bytes);
        if pct != last_pct {
            last_pct = pct;
            emit(received);
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    emit(spec.bytes);
    verify_file(part, spec)
}

/// Delete the model file. The engine keeps its copy until the app quits (it
/// has no unload), so this frees disk, not memory.
#[tauri::command]
pub fn stt_remove_model(app: AppHandle, model: String) -> Result<(), String> {
    let spec = model_spec(&model)?;
    match fs::remove_file(model_path(&models_dir(&app)?, spec)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Dictation switched on: load the model now, so the first pass is quick.
#[tauri::command]
pub async fn stt_load(app: AppHandle, model: String) -> Result<(), String> {
    let spec = model_spec(&model)?;
    let path = model_path(&models_dir(&app)?, spec);
    tauri::async_runtime::spawn_blocking(move || engine::load(spec, &path))
        .await
        .map_err(|e| e.to_string())?
}

fn model_from(request: &Request<'_>) -> Result<&'static ModelSpec, String> {
    let model = request
        .headers()
        .get(MODEL_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or("the speech model wasn't named")?;
    model_spec(model)
}

fn raw_body<'a>(request: &'a Request<'_>) -> Result<&'a [u8], String> {
    match request.body() {
        InvokeBody::Raw(bytes) => Ok(bytes.as_slice()),
        _ => Err("expected raw audio".into()),
    }
}

/// A live dictation pass: 16 kHz mono f32 samples in (raw bytes, model id in
/// a header), phrases with times out, so typed phrases can be trimmed from
/// the audio.
#[tauri::command]
pub async fn stt_transcribe_live(app: AppHandle, request: Request<'_>) -> Result<Vec<Segment>, String> {
    let spec = model_from(&request)?;
    let samples = samples_from_le_bytes(raw_body(&request)?)?;
    let path = model_path(&models_dir(&app)?, spec);
    tauri::async_runtime::spawn_blocking(move || engine::transcribe(spec, &path, &samples))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    // SHA-256 of "hello".
    const HELLO: ModelSpec = ModelSpec {
        id: "test",
        file: "test.bin",
        url: "https://example.invalid/test.bin",
        bytes: 5,
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    };

    fn word(word: &str, start: f64, end: f64) -> Word {
        Word {
            word: word.into(),
            start,
            end,
        }
    }

    fn seg(text: &str, start_ms: i64, end_ms: i64) -> Segment {
        Segment {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn finds_whistle_and_rejects_others() {
        let spec = model_spec("whistle").unwrap();
        assert_eq!(spec.bytes, 16_919_407);
        assert!(spec.url.starts_with("https://huggingface.co/Cactus-Compute/whistle/resolve/"));
        assert!(spec.url.ends_with(spec.file));
        assert!(model_spec("tiny.en").is_err());
        assert!(model_spec("base.en").is_err());
    }

    #[test]
    fn model_checksums_are_sha256_hex() {
        for m in MODELS {
            assert_eq!(m.sha256.len(), 64, "{}", m.id);
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", m.id);
        }
    }

    #[test]
    fn verify_accepts_the_right_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.bin");
        fs::write(&path, b"hello").unwrap();
        assert_eq!(verify_file(&path, &HELLO), Ok(()));
    }

    #[test]
    fn verify_rejects_wrong_content_of_the_right_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.bin");
        fs::write(&path, b"jello").unwrap();
        assert!(verify_file(&path, &HELLO).is_err());
    }

    #[test]
    fn verify_rejects_the_wrong_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.bin");
        fs::write(&path, b"hello!").unwrap();
        assert!(verify_file(&path, &HELLO).unwrap_err().contains("bytes"));
    }

    #[test]
    fn digest_check_ignores_case() {
        assert!(check_digest(&HELLO.sha256.to_uppercase(), &HELLO).is_ok());
    }

    #[test]
    fn legacy_whisper_models_are_removed_and_whistle_kept() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "ggml-tiny.en-q5_1.bin",
            "ggml-base.en-q5_1.bin.part",
            "whistle.cact",
        ] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        remove_legacy(dir.path());
        remove_legacy(dir.path());
        let left: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left, ["whistle.cact"]);
    }

    #[test]
    fn samples_round_trip_from_bytes() {
        let want = [0.5f32, -1.0, 0.25];
        let bytes: Vec<u8> = want.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(samples_from_le_bytes(&bytes).unwrap(), want);
        assert!(samples_from_le_bytes(&bytes[..5]).is_err());
    }

    #[test]
    fn long_clips_split_into_thirty_second_chunks() {
        assert_eq!(chunk_ranges(0, 10), vec![]);
        assert_eq!(chunk_ranges(7, 10), vec![(0, 7)]);
        assert_eq!(chunk_ranges(10, 10), vec![(0, 10)]);
        assert_eq!(chunk_ranges(25, 10), vec![(0, 10), (10, 20), (20, 25)]);
        let two_min = chunk_ranges(120 * SAMPLE_RATE, MAX_CHUNK_SAMPLES);
        assert_eq!(two_min.len(), 4);
        assert!(two_min.iter().all(|(s, e)| e - s <= MAX_CHUNK_SAMPLES));
    }

    #[test]
    fn reads_the_engines_json_up_to_the_nul() {
        let mut out = br#"{"text":"git status","language":"en","words":[{"word":"git","start":0.0,"end":0.24,"probability":0.9},{"word":"status","start":0.24,"end":0.8,"probability":0.99}],"ttft_ms":5.0,"decode_tps":900.0}"#.to_vec();
        out.extend_from_slice(&[0, b'x', b'y']);
        let t = parse_output(&out).unwrap();
        assert_eq!(t.text, "git status");
        assert_eq!(t.words, vec![word("git", 0.0, 0.24), word("status", 0.24, 0.8)]);
    }

    #[test]
    fn silence_parses_to_nothing() {
        let t = parse_output(b"{\"text\":\"\",\"language\":\"\",\"ttft_ms\":0.0,\"decode_tps\":0.0}\0").unwrap();
        assert!(phrases(&t, 0, 2000).is_empty());
    }

    #[test]
    fn rejects_output_without_a_nul_or_with_bad_json() {
        assert!(parse_output(b"{\"text\":\"cut off").unwrap_err().contains("fit"));
        assert!(parse_output(b"{\"text\":\0").unwrap_err().contains("unreadable"));
    }

    #[test]
    fn phrases_break_at_punctuation_and_pauses() {
        let t = Transcript {
            text: String::new(),
            words: vec![
                word("Run", 0.0, 0.24),
                word("tests.", 0.24, 0.64),
                word("Then", 0.72, 0.96),
                word("commit", 0.96, 1.28),
                // 320 ms of silence before "and".
                word("and", 1.6, 1.76),
                word("push", 1.76, 2.0),
            ],
        };
        assert_eq!(
            phrases(&t, 0, 2400),
            vec![
                seg("Run tests.", 0, 640),
                seg("Then commit", 720, 1280),
                seg("and push", 1600, 2000),
            ]
        );
    }

    #[test]
    fn short_gaps_keep_a_phrase_together() {
        let t = Transcript {
            text: String::new(),
            words: vec![word("open", 0.0, 0.3), word("settings", 0.42, 0.9)],
        };
        assert_eq!(phrases(&t, 0, 1000), vec![seg("open settings", 0, 900)]);
    }

    #[test]
    fn later_chunks_are_placed_after_the_earlier_ones() {
        let t = Transcript {
            text: String::new(),
            words: vec![word("hello", 0.5, 1.0)],
        };
        assert_eq!(phrases(&t, 30_000, 4000), vec![seg("hello", 30_500, 31_000)]);
    }

    #[test]
    fn text_without_word_times_is_one_phrase_over_the_chunk() {
        let t = Transcript {
            text: " list files. ".into(),
            words: vec![],
        };
        assert_eq!(phrases(&t, 0, 1500), vec![seg("list files.", 0, 1500)]);
    }

    #[test]
    fn keywords_are_single_lines() {
        for k in KEYWORDS {
            assert!(!k.is_empty() && !k.contains(['\n', '\0']), "{k}");
        }
    }

    #[test]
    fn only_one_download_per_model_at_a_time() {
        let set = Arc::new(Mutex::new(HashSet::new()));
        let first = DownloadGuard::claim(&set, "whistle");
        assert!(first.is_some());
        assert!(DownloadGuard::claim(&set, "whistle").is_none());
        assert!(DownloadGuard::claim(&set, "other").is_some());
        drop(first);
        assert!(DownloadGuard::claim(&set, "whistle").is_some());
    }

    #[test]
    fn progress_is_whole_percent_and_capped() {
        assert_eq!(progress_pct(0, 200), 0);
        assert_eq!(progress_pct(99, 200), 49);
        assert_eq!(progress_pct(200, 200), 100);
        assert_eq!(progress_pct(250, 200), 100);
        assert_eq!(progress_pct(5, 0), 0);
    }

    #[test]
    fn model_path_is_in_the_folder() {
        let p = model_path(Path::new("/tmp/models"), &MODELS[0]);
        assert_eq!(p, Path::new("/tmp/models/whistle.cact"));
    }
}

/// The real engine on the real model. Run in CI with `--ignored`, given
/// WHISTLE_MODEL (the .cact) and optionally WHISTLE_WAV (16 kHz mono 16-bit)
/// with WHISTLE_EXPECT (words that must be heard).
#[cfg(all(test, needle))]
mod engine_tests {
    use super::*;

    fn silence(seconds: usize) -> Vec<f32> {
        std::iter::repeat_n(0.0, seconds * SAMPLE_RATE).collect()
    }

    fn model() -> PathBuf {
        PathBuf::from(std::env::var("WHISTLE_MODEL").expect("set WHISTLE_MODEL"))
    }

    /// The samples of a 16-bit PCM WAV's data chunk.
    fn read_wav(path: &Path) -> Vec<f32> {
        let bytes = fs::read(path).unwrap();
        let mut i = 12;
        while i + 8 <= bytes.len() {
            let id = &bytes[i..i + 4];
            let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
            if id == b"data" {
                return bytes[i + 8..(i + 8 + len).min(bytes.len())]
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                    .collect();
            }
            i += 8 + len + (len & 1);
        }
        panic!("no data chunk in {}", path.display());
    }

    #[test]
    #[ignore]
    fn the_model_verifies_and_loads() {
        verify_file(&model(), &MODELS[0]).unwrap();
        engine::load(&MODELS[0], &model()).unwrap();
        engine::load(&MODELS[0], &model()).unwrap();
    }

    #[test]
    #[ignore]
    fn silence_transcribes_to_nothing() {
        let segs = engine::transcribe(&MODELS[0], &model(), &silence(2)).unwrap();
        assert!(segs.is_empty(), "{segs:?}");
    }

    #[test]
    #[ignore]
    fn over_thirty_seconds_is_chunked_not_refused() {
        let segs = engine::transcribe(&MODELS[0], &model(), &silence(45)).unwrap();
        assert!(segs.is_empty(), "{segs:?}");
    }

    #[test]
    #[ignore]
    fn speech_is_heard() {
        let (Ok(wav), Ok(expect)) = (std::env::var("WHISTLE_WAV"), std::env::var("WHISTLE_EXPECT"))
        else {
            eprintln!("WHISTLE_WAV / WHISTLE_EXPECT not set; skipped");
            return;
        };
        let samples = read_wav(Path::new(&wav));
        let segs = engine::transcribe(&MODELS[0], &model(), &samples).unwrap();
        let heard = segs
            .iter()
            .map(|s| s.text.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("heard: {heard}");
        for w in expect.split_whitespace() {
            assert!(heard.contains(&w.to_lowercase()), "missing {w:?} in {heard:?}");
        }
        assert!(segs.windows(2).all(|p| p[0].end_ms <= p[1].start_ms));
    }
}
