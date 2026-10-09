import { describe, expect, it } from "vitest";
import {
  cleanTranscript,
  dictatedMessage,
  dictationKeys,
  downloadingMessage,
  listeningMessage,
  MODEL,
  readyMessage,
  wordCount,
} from "./text";

describe("cleanTranscript", () => {
  it("drops bracketed markers and collapses whitespace", () => {
    expect(cleanTranscript("  [BLANK_AUDIO] git   status\n")).toBe(
      "git status",
    );
    expect(cleanTranscript("[ Silence ]")).toBe("");
    expect(cleanTranscript("(upbeat music) list the files ♪")).toBe(
      "list the files",
    );
  });

  it("keeps ordinary parentheses", () => {
    expect(cleanTranscript("add a flag (optional) here")).toBe(
      "add a flag (optional) here",
    );
  });

  it("counts words", () => {
    expect(wordCount("git commit -m fix")).toBe(4);
    expect(wordCount("")).toBe(0);
  });
});

describe("wording", () => {
  it("names the keys for the prefix in use", () => {
    expect(dictationKeys("ctrl+b")).toBe("Ctrl+B Ctrl+Space");
    expect(dictationKeys("ctrl+a")).toBe("Ctrl+A Ctrl+Space");
    expect(dictationKeys("ctrl+space")).toBe("Ctrl+Space v");
  });

  it("says what is happening", () => {
    expect(downloadingMessage(42)).toBe(
      "Downloading the speech model (17 MB): 42%.",
    );
    expect(readyMessage("Ctrl+B Ctrl+Space")).toBe(
      "Speech model ready. Press Ctrl+B Ctrl+Space to dictate.",
    );
    expect(listeningMessage("Pane 2", "Ctrl+B Ctrl+Space")).toBe(
      "Listening in pane 2. Ctrl+B Ctrl+Space to stop, Esc to cancel.",
    );
    expect(dictatedMessage(12, "Pane 2")).toBe(
      "Dictated 12 words into pane 2.",
    );
    expect(dictatedMessage(1, "Pane 1")).toBe("Dictated 1 word into pane 1.");
  });

  it("offers Whistle, at its rounded size (16,919,407 bytes in stt.rs)", () => {
    expect(MODEL).toEqual({ id: "whistle", mb: 17 });
  });
});
