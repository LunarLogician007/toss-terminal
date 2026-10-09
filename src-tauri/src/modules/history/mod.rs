mod fixes;
mod parse;

use fixes::{
    fresh_fix, looks_like_fix, nearest_name, verdict, Learned, Verdict, LEARN_WINDOW_SECS,
    MAX_COMMAND_LEN,
};
use parse::{
    build_index, complete_commands, demetafy, list, parse_bash, parse_fish, parse_zsh, sort_recent,
    suggest, HistEntry,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

struct Index {
    entries: Vec<HistEntry>,
    path_cmds: Vec<String>,
}

#[derive(Default)]
pub struct HistoryState {
    inner: Mutex<Option<Index>>,
    /// Which commands worked or failed, and learned corrections (TOSS
    /// Terminal), loaded from the app's data folder on first use.
    learned: Mutex<Option<Learned>>,
    /// Per pane, the last command that failed and when, so the command that
    /// fixes it can be learned.
    last_failure: Mutex<HashMap<u32, (String, i64)>>,
    /// A save is scheduled.
    saving: AtomicBool,
}

const LEARNED_FILE: &str = "command-fixes.json";
/// Commands that only ever failed, one per line, for the zsh integration to
/// keep out of zsh-autosuggestions (TOSS_FAILED_COMMANDS).
const FAILED_FILE: &str = "failed-commands.txt";
const MAX_FAILED_LISTED: usize = 100;
static FAILED_LIST: OnceLock<PathBuf> = OnceLock::new();

/// Note where the failed-commands list lives, before any shell starts.
pub fn init(app: &AppHandle) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = FAILED_LIST.set(dir.join(FAILED_FILE));
    }
}

/// The failed-commands list, for new shells (see `init`).
pub fn failed_list_path() -> Option<&'static Path> {
    FAILED_LIST.get().map(PathBuf::as_path)
}

/// Batch saves: commands often finish in bursts.
const SAVE_DELAY: Duration = Duration::from_secs(2);
/// The end of a failed command's output is where its error and hint are.
const MAX_OUTPUT: usize = 8 * 1024;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn read_histories() -> Vec<(String, i64)> {
    let mut all = Vec::new();
    let home = dirs::home_dir();

    if let Some(path) = zsh_histfile(home.as_ref()) {
        if let Ok(bytes) = std::fs::read(&path) {
            let content = String::from_utf8_lossy(&demetafy(&bytes)).into_owned();
            all.extend(parse_zsh(&content));
        }
    }
    if let Some(home) = home.as_ref() {
        if let Ok(content) = std::fs::read_to_string(home.join(".bash_history")) {
            all.extend(parse_bash(&content));
        }
    }
    if let Some(path) = fish_histfile(home.as_ref()) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            all.extend(parse_fish(&content));
        }
    }
    all
}

fn zsh_histfile(home: Option<&PathBuf>) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HISTFILE") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    home.map(|h| h.join(".zsh_history"))
}

fn fish_histfile(home: Option<&PathBuf>) -> Option<PathBuf> {
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        let pb = PathBuf::from(data).join("fish/fish_history");
        if pb.exists() {
            return Some(pb);
        }
    }
    home.map(|h| h.join(".local/share/fish/fish_history"))
}

fn scan_path() -> Vec<String> {
    use std::collections::HashSet;
    let mut set: HashSet<String> = HashSet::new();
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                if is_executable(&entry) {
                    if let Some(name) = entry.file_name().to_str() {
                        set.insert(name.to_string());
                    }
                }
            }
        }
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

#[cfg(unix)]
fn is_executable(entry: &std::fs::DirEntry) -> bool {
    use std::os::unix::fs::PermissionsExt;
    entry
        .metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(entry: &std::fs::DirEntry) -> bool {
    match entry.file_name().to_str() {
        Some(name) => {
            let lower = name.to_ascii_lowercase();
            [".exe", ".cmd", ".bat", ".com", ".ps1"]
                .iter()
                .any(|e| lower.ends_with(e))
        }
        None => false,
    }
}

fn ensure(state: &HistoryState) -> std::sync::MutexGuard<'_, Option<Index>> {
    let mut guard = state.inner.lock().unwrap();
    if guard.is_none() {
        *guard = Some(Index {
            entries: build_index(read_histories()),
            path_cmds: scan_path(),
        });
    }
    guard
}

fn learned_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(LEARNED_FILE))
}

/// The learned store, loaded on first use (an unreadable file starts over).
fn learned<'a>(app: &AppHandle, state: &'a HistoryState) -> MutexGuard<'a, Option<Learned>> {
    let mut guard = state.learned.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let loaded = learned_path(app)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        *guard = Some(loaded);
    }
    guard
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Write the learned store now, if it was ever loaded.
pub fn save_learned(app: &AppHandle) {
    let Some(state) = app.try_state::<HistoryState>() else {
        return;
    };
    let (json, failed) = {
        let guard = state.learned.lock().unwrap_or_else(|e| e.into_inner());
        let Some(learned) = guard.as_ref() else {
            return;
        };
        let mut failed = learned.failed_list(MAX_FAILED_LISTED).join("\n");
        failed.push('\n');
        (serde_json::to_vec(learned).ok(), failed)
    };
    let mut files = vec![];
    if let (Some(json), Some(path)) = (json, learned_path(app)) {
        files.push((path, json));
    }
    if let Some(path) = failed_list_path() {
        files.push((path.to_path_buf(), failed.into_bytes()));
    }
    for (path, bytes) in files {
        if let Err(e) = write_atomic(&path, &bytes) {
            log::warn!("couldn't save {}: {e}", path.display());
        }
    }
}

fn schedule_save(app: &AppHandle, state: &HistoryState) {
    if state.saving.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SAVE_DELAY);
        if let Some(state) = app.try_state::<HistoryState>() {
            state.saving.store(false, Ordering::SeqCst);
        }
        save_learned(&app);
    });
}

/// A grey suggestion: the rest of a command from history, or (`fix`) the
/// correction of a command that failed before.
#[derive(Debug, PartialEq, Serialize)]
pub struct Suggestion {
    text: String,
    fix: bool,
}

#[tauri::command]
pub fn history_suggest(
    app: AppHandle,
    state: State<'_, HistoryState>,
    line: String,
) -> Option<Suggestion> {
    let guard = ensure(&state);
    let idx = guard.as_ref()?;
    let learned = learned(&app, &state);
    let learned = learned.as_ref()?;
    if let Some(text) = suggest(&idx.entries, &line, |c| learned.only_failed(c)) {
        return Some(Suggestion { text, fix: false });
    }
    learned.fix_for_line(&line).map(|to| Suggestion {
        text: to.to_string(),
        fix: true,
    })
}

#[tauri::command]
pub fn history_commands(
    app: AppHandle,
    state: State<'_, HistoryState>,
    prefix: String,
    limit: Option<usize>,
) -> Vec<String> {
    let guard = ensure(&state);
    let learned = learned(&app, &state);
    match (guard.as_ref(), learned.as_ref()) {
        (Some(idx), Some(learned)) => complete_commands(
            &idx.entries,
            &idx.path_cmds,
            &prefix,
            limit.unwrap_or(50),
            |c| learned.only_failed(c),
        ),
        _ => Vec::new(),
    }
}

/// The last `max` bytes of `s`, cut at a character boundary.
fn tail(s: &str, max: usize) -> &str {
    let mut start = s.len().saturating_sub(max);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

/// The closest program name to a mistyped one: your history's programs
/// (weighted by use, leaving out typos that only failed) and the PATH.
fn nearest_program(idx: &Index, learned: &Learned, typed: &str) -> Option<String> {
    let mut uses: HashMap<&str, u32> = HashMap::new();
    for e in idx.entries.iter().filter(|e| !learned.only_failed(&e.cmd)) {
        if let Some(program) = e.cmd.split_whitespace().next() {
            *uses.entry(program).or_insert(0) += e.count;
        }
    }
    for program in &idx.path_cmds {
        uses.entry(program).or_insert(0);
    }
    nearest_name(typed, uses.into_iter())
}

/// A command finished in pane `pane` (TOSS Terminal): count whether it
/// worked, learn it as the fix of the pane's last failure if it looks like
/// one, and when it failed, return a correction to offer at the next prompt.
/// `output` is the end of what it printed (only sent for failures). Commands
/// typed with a leading space aren't remembered, as in shell history.
#[tauri::command]
pub fn history_finish(
    app: AppHandle,
    state: State<'_, HistoryState>,
    pane: u32,
    command: String,
    exit: Option<i32>,
    output: String,
) -> Option<String> {
    let private = command.starts_with(' ');
    let command = command.trim();
    if command.is_empty() || command.len() > MAX_COMMAND_LEN {
        return None;
    }
    let output = tail(&output, MAX_OUTPUT);
    let verdict = verdict(command, exit);
    let now = now();

    let index = ensure(&state);
    let mut learned_guard = learned(&app, &state);
    let learned = learned_guard.as_mut()?;
    let mut failures = state.last_failure.lock().unwrap_or_else(|e| e.into_inner());
    failures.retain(|_, (_, at)| now - *at <= LEARN_WINDOW_SECS);

    let mut changed = !private && learned.record(command, verdict, now);
    let offer = match verdict {
        Verdict::Worked => {
            let fixes_last = failures
                .get(&pane)
                .is_some_and(|(failed, _)| looks_like_fix(failed, command));
            if fixes_last && !private {
                if let Some((failed, _)) = failures.remove(&pane) {
                    learned.learn(&failed, command, now);
                    changed = true;
                }
            }
            None
        }
        Verdict::Failed => {
            if !private {
                failures.insert(pane, (command.to_string(), now));
            }
            learned.fix_of(command).map(str::to_string).or_else(|| {
                let idx = index.as_ref()?;
                fresh_fix(
                    command,
                    exit,
                    output,
                    |p| idx.path_cmds.binary_search_by(|c| c.as_str().cmp(p)).is_ok(),
                    |p| nearest_program(idx, learned, p),
                )
            })
        }
        Verdict::Neither => None,
    };
    drop(failures);
    drop(learned_guard);
    drop(index);
    if changed {
        schedule_save(&app, &state);
    }
    offer
}

#[tauri::command]
pub fn history_list(
    state: State<'_, HistoryState>,
    query: String,
    limit: Option<usize>,
) -> Vec<String> {
    let guard = ensure(&state);
    match guard.as_ref() {
        Some(idx) => list(&idx.entries, &query, limit.unwrap_or(200)),
        None => Vec::new(),
    }
}

// Called on every accepted command so in-memory history stays hot without a
// re-read. Only ever fed prompt-mode commands, never raw running-mode input,
// so passwords typed into a running command never enter history.
#[tauri::command]
pub fn history_record(state: State<'_, HistoryState>, command: String) {
    let cmd = command.trim();
    if cmd.is_empty() {
        return;
    }
    let mut guard = ensure(&state);
    if let Some(idx) = guard.as_mut() {
        let n = now();
        match idx.entries.iter_mut().find(|e| e.cmd == cmd) {
            Some(e) => {
                e.count += 1;
                e.last = n;
            }
            None => idx.entries.push(HistEntry {
                cmd: cmd.to_string(),
                count: 1,
                last: n,
            }),
        }
        sort_recent(&mut idx.entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end_on_a_char_boundary() {
        assert_eq!(tail("hello", 3), "llo");
        assert_eq!(tail("hi", 10), "hi");
        // "é" is two bytes: a cut inside it moves forward.
        assert_eq!(tail("aé", 1), "");
        assert_eq!(tail("aéb", 2), "b");
    }

    #[test]
    fn nearest_program_uses_history_and_path_but_not_failed_typos() {
        let idx = Index {
            entries: build_index(vec![
                ("gst".into(), 1),
                ("gst".into(), 2),
                ("gtis x".into(), 3),
            ]),
            path_cmds: vec!["git".into(), "gist".into()],
        };
        let mut learned = Learned::default();
        learned.record("gtis x", Verdict::Failed, 1);
        assert_eq!(nearest_program(&idx, &learned, "gti"), Some("git".into()));
        // An alias only your history knows.
        assert_eq!(nearest_program(&idx, &learned, "gts"), Some("gst".into()));
        // "gtis" would be one edit away, but it only ever failed.
        assert_eq!(nearest_program(&idx, &learned, "gtix"), None);
    }
}
