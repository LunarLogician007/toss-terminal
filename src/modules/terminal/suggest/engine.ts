import type { Suggestion } from "../block/lib/history";
import { applyInput, FRESH, type LineState } from "./lineModel";

/** The part of a suggested command not typed yet, or null. */
export function suffixFor(line: string, full: string): string | null {
  return full.length > line.length && full.startsWith(line)
    ? full.slice(line.length)
    : null;
}

/** One word of a suggestion: leading blanks, then up to the next blank. */
export function nextWord(suffix: string): string {
  return suffix.match(/^\s*\S*/)?.[0] || suffix;
}

/**
 * What a suggestion offers for the line typed so far. Either the rest of the
 * line (`replace` false, drawn as is), or, for a correction that doesn't
 * continue what's typed, a whole new line (`replace` true, drawn after an
 * arrow). `text` is what to add, or the new line.
 */
export type Offer = {
  draw: string;
  text: string;
  replace: boolean;
  fix: boolean;
};

export const FIX_ARROW = "→ ";

export function offerFor(line: string, s: Suggestion): Offer | null {
  const rest = suffixFor(line, s.text);
  if (!s.fix) {
    return rest ? { draw: rest, text: rest, replace: false, fix: false } : null;
  }
  if (line === "") {
    return {
      draw: FIX_ARROW + s.text,
      text: s.text,
      replace: false,
      fix: true,
    };
  }
  if (rest) return { draw: rest, text: rest, replace: false, fix: true };
  if (s.text === line) return null;
  return {
    draw: `  ${FIX_ARROW}${s.text}`,
    text: s.text,
    replace: true,
    fix: true,
  };
}

/** Whether a suggestion still fits the line (continues it), so it can stay. */
function fits(line: string, s: Suggestion): boolean {
  return (s.fix && line === "") || suffixFor(line, s.text) !== null;
}

/**
 * Keys that take a visible suggestion, as in fish and zsh-autosuggestions:
 * Right, End or Ctrl+F take it all; Ctrl+Right or Option/Alt+Right one word.
 */
export function acceptKind(
  e: Pick<KeyboardEvent, "key" | "ctrlKey" | "altKey" | "metaKey" | "shiftKey">,
): "all" | "word" | null {
  if (e.shiftKey || e.metaKey) return null;
  if (e.key === "ArrowRight") {
    if (e.ctrlKey || e.altKey) return "word";
    return "all";
  }
  if (e.ctrlKey || e.altKey) return e.ctrlKey && e.key === "f" ? "all" : null;
  return e.key === "End" ? "all" : null;
}

export type SuggestDeps = {
  /** The "Command suggestions" setting isn't Off. */
  enabled: () => boolean;
  /** A command from history starting with the line, or a correction. */
  suggest: (line: string) => Promise<Suggestion | null>;
  /** Remember a command that ran, so it can be suggested. */
  record: (command: string) => void;
  /**
   * A command finished with this exit status (null when unknown): learn
   * from it. Resolves to a correction to offer when it failed.
   */
  finish: (command: string, exit: number | null) => Promise<string | null>;
  /** Draw the grey text after the cursor, or hide it (null). */
  show: (offer: Offer | null) => void;
};

/** Longest command the shell's own report of it is trusted for. */
const SHELL_COMMAND_MAX = 255;

/**
 * Command suggestions for one terminal pane (TOSS Terminal). Fed the pane's
 * keystrokes and its shell's prompt state; shows a history suggestion while
 * the typed line is known, and none otherwise. When a command fails, offers
 * its correction at the next prompt.
 */
export function createSuggestEngine(deps: SuggestDeps) {
  let line: LineState = FRESH;
  let atPrompt = false;
  let shellSuggests = false;
  let full: Suggestion | null = null;
  let seq = 0;
  /** The command now running, as typed (or as the shell reported it). */
  let running: string | null = null;
  /** Enter was pressed and the shell hasn't started a command since. */
  let awaitingStart = false;
  /** Bumped per command, so a late correction for an old one is dropped. */
  let commandSeq = 0;

  /** Something may be drawn: at a prompt, with the line known. */
  const active = () => deps.enabled() && atPrompt && line.certain;

  /**
   * With the shell's own suggestions on (zsh-autosuggestions, fish), only a
   * correction on the empty line is offered: there the shell draws nothing,
   * and once you type, its grey text is the one shown.
   */
  const fixOnly = () => shellSuggests;

  const current = (): Offer | null => {
    if (!active() || full === null) return null;
    if (fixOnly() && !(full.fix && line.text === "")) return null;
    return offerFor(line.text, full);
  };

  function hide() {
    full = null;
    deps.show(null);
  }

  function refresh() {
    const text = line.text;
    if (!active()) {
      seq++;
      hide();
      return;
    }
    if (full?.fix && fits(text, full) && !(fixOnly() && text !== "")) {
      // A correction being typed out stays ahead of history.
      seq++;
      deps.show(current());
      return;
    }
    if (fixOnly()) {
      seq++;
      hide();
      return;
    }
    if (!text.trim()) {
      seq++;
      hide();
      return;
    }
    // Keep showing the current suggestion while it still fits what's typed.
    if (full && fits(text, full)) deps.show(current());
    else hide();
    const mine = ++seq;
    void deps.suggest(text).then((next) => {
      if (mine !== seq || !active() || line.text !== text) return;
      const offer = next ? offerFor(text, next) : null;
      if (next && offer) {
        full = next;
        deps.show(offer);
      } else {
        hide();
      }
    });
  }

  return {
    /** From the shell integration: false = at a prompt, true = running. */
    promptState(isRunning: boolean) {
      // A prompt drawn right after a command ended (OSC 133 A after D)
      // keeps a correction already offered on the empty line.
      if (!isRunning && atPrompt && line.text === "" && line.certain) return;
      atPrompt = !isRunning;
      line = FRESH;
      seq++;
      hide();
    },
    /** The shell started a command (OSC 133 C), naming it when it can. */
    commandStarted(command: string) {
      if (!awaitingStart) return;
      awaitingStart = false;
      // The shell's name for it covers a line edited out of sight (Tab).
      if (running === null && command && command.length < SHELL_COMMAND_MAX) {
        running = command;
      }
    },
    /** The running command ended (OSC 133 D) with this exit status. */
    commandFinished(exit: number | null) {
      awaitingStart = false;
      const command = running;
      running = null;
      if (command === null) return;
      const mine = ++commandSeq;
      void deps.finish(command, exit).then((fix) => {
        if (!fix || mine !== commandSeq || !active() || line.text !== "") {
          return;
        }
        full = { text: fix, fix: true };
        seq++;
        deps.show(current());
      });
    },
    /** Keystrokes and pastes on their way to the shell. */
    input(data: string) {
      const r = applyInput(line, data);
      line = r.state;
      // An Enter inside a running program (a password prompt) is its own
      // business: the command being run stays as it was.
      if (r.entered && atPrompt) {
        commandSeq++;
        running = r.submitted ?? null;
        awaitingStart = true;
      }
      // Only lines run at a prompt: never what's typed into a running
      // program (a password prompt).
      if (r.submitted && atPrompt) deps.record(r.submitted.trim());
      refresh();
    },
    /** The shell has suggestions of its own (zsh-autosuggestions, fish). */
    markShellSuggests() {
      shellSuggests = true;
      seq++;
      deps.show(current());
    },
    /**
     * Take the suggestion (all or one word); the text to send, or null. A
     * correction that replaces the line erases what's typed first.
     */
    accept(kind: "all" | "word"): string | null {
      const offer = current();
      if (!offer) return null;
      if (offer.replace) {
        const erase = "\x7f".repeat(Array.from(line.text).length);
        line = { text: offer.text, certain: true };
        refresh();
        return erase + offer.text;
      }
      const taken = kind === "all" ? offer.text : nextWord(offer.text);
      line = { text: line.text + taken, certain: true };
      refresh();
      return taken;
    },
    /** What's drawn now, if anything (to redraw it after the cursor moves). */
    visible(): Offer | null {
      return current();
    },
  };
}

export type SuggestEngine = ReturnType<typeof createSuggestEngine>;
