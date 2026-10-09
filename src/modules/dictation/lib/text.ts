import type { PrefixKey } from "@/modules/tiling/lib/prefix";

export type ModelId = "whistle";

/** The built-in model (Cactus Compute's Whistle); the size matches stt.rs. */
export const MODEL: { id: ModelId; mb: number } = { id: "whistle", mb: 17 };

// Non-speech annotations a model may emit: "[BLANK_AUDIO]", "(music)", "♪".
const BRACKETED = /\[[^\]]*\]/g;
const NON_SPEECH =
  /\([^)]*\b(?:music|silence|applause|laugh\w*|sigh\w*|cough\w*|noise|inaudible|breathing|typing|clicking|wind)\b[^)]*\)/gi;

/** What was heard, minus non-speech markers, on one line. */
export function cleanTranscript(raw: string): string {
  return raw
    .replace(BRACKETED, " ")
    .replace(NON_SPEECH, " ")
    .replace(/[♪♫]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function wordCount(text: string): number {
  return text.split(/\s+/).filter(Boolean).length;
}

const PREFIX_LABEL: Record<PrefixKey, string> = {
  "ctrl+b": "Ctrl+B",
  "ctrl+a": "Ctrl+A",
  "ctrl+space": "Ctrl+Space",
};

/** The keys that start and stop dictation, for the prefix in use. */
export function dictationKeys(prefix: PrefixKey): string {
  // With Ctrl+Space as the prefix, pressing it twice sends it to the shell.
  return `${PREFIX_LABEL[prefix]} ${prefix === "ctrl+space" ? "v" : "Ctrl+Space"}`;
}

export function downloadingMessage(pct: number): string {
  return `Downloading the speech model (${MODEL.mb} MB): ${pct}%.`;
}

export function readyMessage(keys: string): string {
  return `Speech model ready. Press ${keys} to dictate.`;
}

export function listeningMessage(pane: string, keys: string): string {
  return `Listening in ${pane.toLowerCase()}. ${keys} to stop, Esc to cancel.`;
}

export function dictatedMessage(words: number, pane: string): string {
  return `Dictated ${words} ${words === 1 ? "word" : "words"} into ${pane.toLowerCase()}.`;
}

/** While listening, with the words not settled on yet. */
export function liveMessage(pane: string, tail: string[]): string {
  return `Listening in ${pane.toLowerCase()}: …${tail.slice(-8).join(" ")}`;
}
