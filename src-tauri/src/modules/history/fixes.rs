// TOSS Terminal: command corrections. Which commands worked, which failed,
// and what a failed command should have been: learned from you (a failure
// followed by a near-identical command that worked) or worked out on the spot
// (the closest installed command, or the tool's own "did you mean" hint).
// Pure: no I/O, so every rule is unit-tested.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Most commands remembered; the least recently run go first.
pub const MAX_OUTCOMES: usize = 5000;
/// Most corrections remembered.
pub const MAX_FIXES: usize = 500;
/// Longer command lines aren't remembered (or worth correcting).
pub const MAX_COMMAND_LEN: usize = 1000;
/// A fix must come this soon after the failure to be learned from.
pub const LEARN_WINDOW_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Worked,
    Failed,
    /// Interrupted, unknown, or a "no" that isn't an error (grep found
    /// nothing): neither counts.
    Neither,
}

/// Commands whose exit 1 means "no" (no match, files differ), not "error".
const ANSWERS_WITH_ONE: &[&str] = &[
    "grep", "egrep", "fgrep", "rg", "ag", "ack", "diff", "cmp", "test", "[", "[[", "false",
    "which", "type", "command", "pgrep", "pidof", "git diff", "git grep",
];

fn answers_with_one(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let first = words.next().unwrap_or("");
    let two = format!("{first} {}", words.next().unwrap_or(""));
    ANSWERS_WITH_ONE
        .iter()
        .any(|c| *c == first || *c == two.trim_end())
}

/// What an exit status says about a command. 128 and up is a signal (Ctrl+C
/// is 130, Ctrl+Z 146 or 148): you stopped it, it didn't fail.
pub fn verdict(command: &str, exit: Option<i32>) -> Verdict {
    match exit {
        Some(0) => Verdict::Worked,
        Some(1) if answers_with_one(command) => Verdict::Neither,
        Some(code) if (1..128).contains(&code) => Verdict::Failed,
        _ => Verdict::Neither,
    }
}

/// Optimal string alignment distance (Levenshtein plus swapped neighbours),
/// on characters: "gti" is one away from "git".
#[allow(clippy::needless_range_loop)]
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[n][m]
}

/// A typo, not a different word: one edit for short words, two for longer.
pub fn is_typo_of(typed: &str, meant: &str) -> bool {
    if typed == meant || typed.is_empty() || meant.is_empty() {
        return false;
    }
    let shorter = typed.chars().count().min(meant.chars().count());
    let allowed = if shorter <= 4 { 1 } else { 2 };
    distance(typed, meant) <= allowed
}

/// Whether `fixed` reads as a correction of `failed`, not a different
/// command: a typo in one word, an added flag, or the same thing with sudo.
pub fn looks_like_fix(failed: &str, fixed: &str) -> bool {
    let (failed, fixed) = (failed.trim(), fixed.trim());
    if failed.is_empty() || failed == fixed {
        return false;
    }
    if fixed.strip_prefix("sudo ").map(str::trim_start) == Some(failed) {
        return true;
    }
    let f: Vec<&str> = failed.split_whitespace().collect();
    let s: Vec<&str> = fixed.split_whitespace().collect();
    if f.len() == s.len() {
        let differ: Vec<usize> = (0..f.len()).filter(|&i| f[i] != s[i]).collect();
        return differ.len() == 1 && is_typo_of(f[differ[0]], s[differ[0]]);
    }
    if s.len() == f.len() + 1 && f.first() == s.first() {
        // One added flag: `git commit "msg"` → `git commit -m "msg"`.
        return (1..s.len()).any(|i| {
            s[i].starts_with('-') && s[..i].iter().chain(&s[i + 1..]).eq(f.iter())
        });
    }
    false
}

/// A word from a tool's output that is safe to put on a command line.
fn safe_word(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 64
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@+,%~^".contains(c))
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    for (open, close) in [('`', '`'), ('`', '\''), ('\'', '\''), ('"', '"')] {
        if let Some(inner) = s.strip_prefix(open).and_then(|r| r.split(close).next()) {
            return inner;
        }
    }
    s
}

fn line_after<'a>(output: &'a str, marker: &str) -> Option<&'a str> {
    let at = output.find(marker)? + marker.len();
    output[at..]
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|l| !l.is_empty())
}

/// The suggestion in a tool's error output, as words: git's "The most
/// similar command is", cargo's "Did you mean `build`?", npm's "Did you mean
/// this?" with the command on the next line.
pub fn hint_from_output(output: &str) -> Option<Vec<String>> {
    let raw: &str = if let Some(next) = line_after(output, "The most similar command")
    {
        next
    } else if let Some(next) = line_after(output, "Did you mean this?")
        .or_else(|| line_after(output, "Did you mean one of these?"))
    {
        next.split('#').next().unwrap_or(next)
    } else {
        let at = output.find("Did you mean ")? + "Did you mean ".len();
        let rest = &output[at..];
        unquote(rest.split(['?', '\n']).next().unwrap_or(rest))
    };
    let words: Vec<String> = raw.split_whitespace().map(str::to_string).collect();
    (!words.is_empty() && words.iter().all(|w| safe_word(w))).then_some(words)
}

/// The command with a hint applied: one word replaces the word it is a typo
/// of; a whole command must start with the same program and stay close.
pub fn apply_hint(command: &str, hint: &[String]) -> Option<String> {
    let words: Vec<&str> = command.split_whitespace().collect();
    match hint {
        [] => None,
        [word] => {
            let (at, _) = words
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, w)| is_typo_of(w, word) || distance(w, word) <= 2)
                .min_by_key(|(_, w)| distance(w, word))?;
            let mut out = words;
            out[at] = word.as_str();
            Some(out.join(" "))
        }
        whole => {
            let same_program = words.first().copied() == whole.first().map(String::as_str);
            let close = whole.len() <= words.len() + 2;
            (same_program && close).then(|| {
                // Keep the arguments the hint left out ("npm install" for
                // "npm isntall lodash").
                let mut out: Vec<String> = whole.to_vec();
                if whole.len() <= words.len() {
                    out.extend(words[whole.len()..].iter().map(|w| w.to_string()));
                }
                out.join(" ")
            })
        }
    }
}

/// A fix for a command that just failed, worked out on the spot: the closest
/// known command when the shell couldn't find the program (exit 127), else
/// the tool's own hint. `nearest` finds the closest known program name.
pub fn fresh_fix(
    command: &str,
    exit: Option<i32>,
    output: &str,
    is_program: impl Fn(&str) -> bool,
    nearest: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut words = command.split_whitespace();
    let program = words.next()?;
    let fix = if exit == Some(127) && !is_program(program) {
        let meant = nearest(program)?;
        std::iter::once(meant.as_str())
            .chain(words)
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        apply_hint(command, &hint_from_output(output)?)?
    };
    (fix != command.trim()).then_some(fix)
}

/// The closest name to `typed` among `names` (with how often each is used):
/// fewest edits first, then the most used, then alphabetical.
pub fn nearest_name<'a>(
    typed: &str,
    names: impl Iterator<Item = (&'a str, u32)>,
) -> Option<String> {
    names
        .filter(|(n, _)| is_typo_of(typed, n))
        .map(|(n, uses)| (distance(typed, n), std::cmp::Reverse(uses), n))
        .min()
        .map(|(_, _, n)| n.to_string())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub worked: u32,
    pub failed: u32,
    pub last: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    pub to: String,
    pub count: u32,
    pub last: i64,
}

/// What TOSS Terminal has learned about your commands. Saved as JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Learned {
    #[serde(default)]
    pub outcomes: HashMap<String, Outcome>,
    #[serde(default)]
    pub fixes: HashMap<String, Fix>,
}

fn keep_newest<V>(map: &mut HashMap<String, V>, max: usize, last: impl Fn(&V) -> i64) {
    if map.len() <= max {
        return;
    }
    let mut ages: Vec<i64> = map.values().map(&last).collect();
    ages.sort_unstable_by(|a, b| b.cmp(a));
    let cutoff = ages[max - 1];
    map.retain(|_, v| last(v) >= cutoff);
    // Ties at the cutoff can leave a few over; drop them in any order.
    while map.len() > max {
        let Some(key) = map.keys().next().cloned() else {
            break;
        };
        map.remove(&key);
    }
}

impl Learned {
    /// Count a finished command. False when nothing changed.
    pub fn record(&mut self, command: &str, verdict: Verdict, now: i64) -> bool {
        if verdict == Verdict::Neither || command.len() > MAX_COMMAND_LEN {
            return false;
        }
        let o = self.outcomes.entry(command.to_string()).or_default();
        match verdict {
            Verdict::Worked => o.worked += 1,
            Verdict::Failed => o.failed += 1,
            Verdict::Neither => {}
        }
        o.last = now;
        if verdict == Verdict::Worked {
            // It works now, so it needs no correcting.
            self.fixes.remove(command);
        }
        keep_newest(&mut self.outcomes, MAX_OUTCOMES, |o| o.last);
        true
    }

    /// Remember that `failed` should have been `fixed`.
    pub fn learn(&mut self, failed: &str, fixed: &str, now: i64) {
        if failed.len() > MAX_COMMAND_LEN || fixed.len() > MAX_COMMAND_LEN {
            return;
        }
        let fix = self.fixes.entry(failed.to_string()).or_insert(Fix {
            to: fixed.to_string(),
            count: 0,
            last: now,
        });
        if fix.to != fixed {
            fix.to = fixed.to_string();
            fix.count = 0;
        }
        fix.count += 1;
        fix.last = now;
        keep_newest(&mut self.fixes, MAX_FIXES, |f| f.last);
    }

    /// Has failed every time it was run here: never suggest it.
    pub fn only_failed(&self, command: &str) -> bool {
        self.outcomes
            .get(command)
            .is_some_and(|o| o.failed > 0 && o.worked == 0)
    }

    /// Commands that only ever failed, newest first, for zsh-autosuggestions
    /// to skip. One line each, so multi-line commands are left out.
    pub fn failed_list(&self, max: usize) -> Vec<&str> {
        let mut failed: Vec<(&str, i64)> = self
            .outcomes
            .iter()
            .filter(|(cmd, o)| o.worked == 0 && o.failed > 0 && !cmd.contains(['\n', '\r', '\0']))
            .map(|(cmd, o)| (cmd.as_str(), o.last))
            .collect();
        failed.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        failed.into_iter().take(max).map(|(cmd, _)| cmd).collect()
    }

    /// The learned correction of exactly this command.
    pub fn fix_of(&self, command: &str) -> Option<&str> {
        self.fixes.get(command).map(|f| f.to.as_str())
    }

    /// The correction for a line being typed: of a failed command that starts
    /// with it (the most used, then the newest).
    pub fn fix_for_line(&self, line: &str) -> Option<&str> {
        if line.trim().chars().count() < 2 {
            return None;
        }
        self.fixes
            .iter()
            .filter(|(from, _)| from.starts_with(line))
            .max_by_key(|(from, f)| (*from == line, f.count, f.last))
            .map(|(_, f)| f.to.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn exit_codes_become_verdicts() {
        assert_eq!(verdict("git push", Some(0)), Verdict::Worked);
        assert_eq!(verdict("git psuh", Some(1)), Verdict::Failed);
        assert_eq!(verdict("gti status", Some(127)), Verdict::Failed);
        assert_eq!(verdict("sleep 9", Some(130)), Verdict::Neither);
        assert_eq!(verdict("vim", Some(148)), Verdict::Neither);
        assert_eq!(verdict("ls", None), Verdict::Neither);
    }

    #[test]
    fn a_no_from_grep_or_diff_is_not_a_failure() {
        assert_eq!(verdict("grep foo x.txt", Some(1)), Verdict::Neither);
        assert_eq!(verdict("git diff --quiet", Some(1)), Verdict::Neither);
        assert_eq!(verdict("grep -r", Some(2)), Verdict::Failed);
        assert_eq!(verdict("git push", Some(1)), Verdict::Failed);
    }

    #[test]
    fn distance_counts_swaps_as_one() {
        assert_eq!(distance("gti", "git"), 1);
        assert_eq!(distance("psuh", "push"), 1);
        assert_eq!(distance("instal", "install"), 1);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "ab"), 2);
        assert_eq!(distance("é", "e"), 1);
    }

    #[test]
    fn short_words_allow_one_edit_longer_two() {
        assert!(is_typo_of("gti", "git"));
        assert!(!is_typo_of("go", "git"));
        assert!(is_typo_of("chekcotu", "checkout"));
        assert!(!is_typo_of("make", "vim"));
        assert!(!is_typo_of("git", "git"));
    }

    #[test]
    fn fixes_are_typos_flags_or_sudo() {
        assert!(looks_like_fix("gti status", "git status"));
        assert!(looks_like_fix("git psuh", "git push"));
        assert!(looks_like_fix("git commit \"x\"", "git commit -m \"x\""));
        assert!(looks_like_fix("apt install vim", "sudo apt install vim"));
    }

    #[test]
    fn different_commands_are_not_fixes() {
        assert!(!looks_like_fix("make", "vim Makefile"));
        assert!(!looks_like_fix("git push", "git push"));
        assert!(!looks_like_fix("git push", "git pull origin main"));
        assert!(!looks_like_fix("npm test", "npm run build"));
        assert!(!looks_like_fix("git checkout a", "git rebase b"));
        assert!(!looks_like_fix("ls", "ls src"));
    }

    #[test]
    fn reads_gits_hint() {
        let out = "git: 'psuh' is not a git command. See 'git --help'.\n\nThe most similar command is\n\tpush\n";
        assert_eq!(hint_from_output(out), Some(strings(&["push"])));
        let two = "The most similar commands are\n\tstash\n\tstatus\n";
        assert_eq!(hint_from_output(two), Some(strings(&["stash"])));
    }

    #[test]
    fn reads_cargo_and_npm_hints() {
        let cargo = "error: no such command: `biuld`\n\n\tDid you mean `build`?\n";
        assert_eq!(hint_from_output(cargo), Some(strings(&["build"])));
        let npm = "Unknown command: \"isntall\"\n\nDid you mean this?\n    npm install # Install a package\n";
        assert_eq!(hint_from_output(npm), Some(strings(&["npm", "install"])));
        let pnpm = "Command \"biuld\" not found. Did you mean \"pnpm run build\"?";
        assert_eq!(hint_from_output(pnpm), Some(strings(&["pnpm", "run", "build"])));
    }

    #[test]
    fn hints_with_shell_syntax_are_refused() {
        assert_eq!(hint_from_output("Did you mean `push; rm -rf ~`?"), None);
        assert_eq!(hint_from_output("Did you mean `$(curl x)`?"), None);
        assert_eq!(hint_from_output("all good"), None);
    }

    #[test]
    fn a_one_word_hint_replaces_its_typo() {
        assert_eq!(
            apply_hint("git psuh origin main", &strings(&["push"])),
            Some("git push origin main".into())
        );
        assert_eq!(apply_hint("cargo biuld --release", &strings(&["build"])), Some("cargo build --release".into()));
        // Nothing in the command is close to the hint.
        assert_eq!(apply_hint("git frobnicate", &strings(&["push"])), None);
    }

    #[test]
    fn a_whole_command_hint_keeps_the_arguments() {
        assert_eq!(
            apply_hint("npm isntall lodash", &strings(&["npm", "install"])),
            Some("npm install lodash".into())
        );
        assert_eq!(
            apply_hint("pnpm biuld", &strings(&["pnpm", "run", "build"])),
            Some("pnpm run build".into())
        );
        assert_eq!(apply_hint("yarn x", &strings(&["npm", "install"])), None);
    }

    #[test]
    fn a_missing_program_gets_the_closest_known_one() {
        let known = ["git", "grep", "gzip", "ls", "python3"];
        let is_program = |p: &str| known.contains(&p);
        let nearest = |p: &str| nearest_name(p, known.iter().map(|n| (*n, 1)));
        assert_eq!(
            fresh_fix("gti status", Some(127), "", is_program, nearest),
            Some("git status".into())
        );
        assert_eq!(fresh_fix("sl -la", Some(127), "", is_program, nearest), Some("ls -la".into()));
        assert_eq!(fresh_fix("qqqqq", Some(127), "", is_program, nearest), None);
        // Found, so not a typo of the program; no hint either.
        assert_eq!(fresh_fix("git status", Some(127), "", is_program, nearest), None);
    }

    #[test]
    fn fixes_git_from_what_the_terminal_holds() {
        // The pane's last lines when `git psuh` failed under Powerlevel10k:
        // the tab before "push" is spaces in the terminal's buffer.
        let out = "  ~ ····························  base ─╮\n❯ git psuh                                  ─╯\ngit: 'psuh' is not a git command. See 'git --help'.\n\nThe most similar command is\n        push\n";
        assert_eq!(hint_from_output(out), Some(strings(&["push"])));
        assert_eq!(
            fresh_fix("git psuh", Some(1), out, |_| true, |_| None),
            Some("git push".into())
        );
    }

    #[test]
    fn a_tools_hint_fixes_its_subcommand() {
        let out = "The most similar command is\n\tpush\n";
        assert_eq!(
            fresh_fix("git psuh", Some(1), out, |_| true, |_| None),
            Some("git push".into())
        );
    }

    #[test]
    fn nearest_prefers_fewer_edits_then_more_use() {
        let names = [("gist", 1), ("git", 50), ("gti2", 0)];
        assert_eq!(nearest_name("gti", names.into_iter()), Some("git".into()));
        let tied = [("cat", 1), ("cut", 9)];
        assert_eq!(nearest_name("cbt", tied.into_iter()), Some("cut".into()));
        assert_eq!(nearest_name("zzz", names.into_iter()), None);
    }

    #[test]
    fn only_failed_commands_are_hidden() {
        let mut l = Learned::default();
        l.record("git psuh", Verdict::Failed, 1);
        assert!(l.only_failed("git psuh"));
        l.record("make", Verdict::Failed, 2);
        l.record("make", Verdict::Worked, 3);
        assert!(!l.only_failed("make"));
        assert!(!l.only_failed("never run"));
        assert!(!l.record("sleep 9", Verdict::Neither, 4));
    }

    #[test]
    fn lists_only_failed_commands_newest_first() {
        let mut l = Learned::default();
        l.record("gti status", Verdict::Failed, 1);
        l.record("git psuh", Verdict::Failed, 3);
        l.record("make", Verdict::Failed, 4);
        l.record("make", Verdict::Worked, 5);
        l.record("echo 'a\nb'\nfalse", Verdict::Failed, 6);
        assert_eq!(l.failed_list(10), vec!["git psuh", "gti status"]);
        assert_eq!(l.failed_list(1), vec!["git psuh"]);
    }

    #[test]
    fn learned_fixes_are_found_by_what_you_type() {
        let mut l = Learned::default();
        l.learn("gti status", "git status", 10);
        assert_eq!(l.fix_of("gti status"), Some("git status"));
        assert_eq!(l.fix_for_line("gti st"), Some("git status"));
        assert_eq!(l.fix_for_line("g"), None);
        assert_eq!(l.fix_for_line("git"), None);
    }

    #[test]
    fn a_command_that_starts_working_loses_its_fix() {
        let mut l = Learned::default();
        l.learn("mk build", "make build", 1);
        l.record("mk build", Verdict::Worked, 2);
        assert_eq!(l.fix_of("mk build"), None);
    }

    #[test]
    fn relearning_a_different_fix_replaces_it() {
        let mut l = Learned::default();
        l.learn("gco", "git checkout", 1);
        l.learn("gco", "git commit", 2);
        assert_eq!(l.fixes["gco"], Fix { to: "git commit".into(), count: 1, last: 2 });
        l.learn("gco", "git commit", 3);
        assert_eq!(l.fixes["gco"].count, 2);
    }

    #[test]
    fn the_store_is_capped_newest_kept() {
        let mut l = Learned::default();
        for i in 0..(MAX_OUTCOMES as i64 + 10) {
            l.record(&format!("cmd {i}"), Verdict::Worked, i);
        }
        assert_eq!(l.outcomes.len(), MAX_OUTCOMES);
        assert!(l.outcomes.contains_key(&format!("cmd {}", MAX_OUTCOMES + 9)));
        assert!(!l.outcomes.contains_key("cmd 0"));
        assert!(!l.record(&"x".repeat(MAX_COMMAND_LEN + 1), Verdict::Worked, 1));
    }

    #[test]
    fn saves_and_loads_as_json() {
        let mut l = Learned::default();
        l.record("git push", Verdict::Worked, 5);
        l.learn("git psuh", "git push", 6);
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<Learned>(&json).unwrap(), l);
        assert_eq!(serde_json::from_str::<Learned>("{}").unwrap(), Learned::default());
    }
}
