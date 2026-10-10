import { describe, expect, it } from "vitest";
import {
  acceptKind,
  createSuggestEngine,
  nextWord,
  offerFor,
  suffixFor,
} from "./engine";

const flush = () => new Promise((r) => setTimeout(r, 0));
const key = (k: string, mods: Partial<KeyboardEvent> = {}) =>
  ({
    key: k,
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    shiftKey: false,
    ...mods,
  }) as KeyboardEvent;

function setup(
  history: Record<string, string> = {
    g: "git status",
    "git c": "git commit -m fix",
  },
  fixes: Record<string, string> = {},
) {
  const shown: (string | null)[] = [];
  const recorded: string[] = [];
  const finished: [string, number | null][] = [];
  let enabled = true;
  const e = createSuggestEngine({
    enabled: () => enabled,
    suggest: async (line) => {
      const hit = Object.entries(history)
        .filter(([p]) => line.startsWith(p))
        .sort((a, b) => b[0].length - a[0].length)[0];
      if (hit?.[1].startsWith(line)) return { text: hit[1], fix: false };
      const fix = Object.entries(fixes).find(([from]) => from.startsWith(line));
      return fix ? { text: fix[1], fix: true } : null;
    },
    record: (c) => recorded.push(c),
    finish: async (command, exit) => {
      finished.push([command, exit]);
      return exit ? (fixes[command] ?? null) : null;
    },
    show: (o) => shown.push(o?.draw ?? null),
  });
  /** Run a command the way the shell reports it: C, then D with `exit`. */
  const run = async (typed: string, exit: number, shellSays = "") => {
    e.input(`${typed}\r`);
    e.promptState(true);
    e.commandStarted(shellSays);
    e.promptState(false);
    e.commandFinished(exit);
    e.promptState(false); // OSC 133 A, the next prompt
    await flush();
  };
  return {
    e,
    shown,
    recorded,
    finished,
    run,
    last: () => shown[shown.length - 1],
    setEnabled: (v: boolean) => {
      enabled = v;
    },
  };
}

describe("text helpers", () => {
  it("suffixFor gives the untyped rest, or nothing", () => {
    expect(suffixFor("git st", "git status")).toBe("atus");
    expect(suffixFor("git status", "git status")).toBeNull();
    expect(suffixFor("ls", "git status")).toBeNull();
  });

  it("nextWord takes the leading blanks and one word", () => {
    expect(nextWord(" commit -m fix")).toBe(" commit");
    expect(nextWord("mit -m fix")).toBe("mit");
    expect(nextWord("x")).toBe("x");
  });

  it("acceptKind: Right/End/Ctrl+F take it all, Ctrl or Option+Right one word", () => {
    expect(acceptKind(key("ArrowRight"))).toBe("all");
    expect(acceptKind(key("End"))).toBe("all");
    expect(acceptKind(key("f", { ctrlKey: true }))).toBe("all");
    expect(acceptKind(key("ArrowRight", { ctrlKey: true }))).toBe("word");
    expect(acceptKind(key("ArrowRight", { altKey: true }))).toBe("word");
    expect(acceptKind(key("ArrowRight", { shiftKey: true }))).toBeNull();
    expect(acceptKind(key("a"))).toBeNull();
  });
});

describe("suggest engine", () => {
  it("suggests from history as you type at a prompt", async () => {
    const t = setup();
    t.e.promptState(false);
    t.e.input("g");
    await flush();
    expect(t.last()).toBe("it status");
    t.e.input("i");
    expect(t.last()).toBe("t status"); // narrowed at once, before history answers
  });

  it("says nothing while a command runs, or before any prompt", async () => {
    const t = setup();
    t.e.input("g");
    await flush();
    expect(t.shown.filter(Boolean)).toEqual([]);
    t.e.promptState(false);
    t.e.promptState(true);
    t.e.input("g");
    await flush();
    expect(t.shown.filter(Boolean)).toEqual([]);
  });

  it("hides once the line is unsure (Tab, history, cursor moves)", async () => {
    const t = setup();
    t.e.promptState(false);
    t.e.input("g");
    await flush();
    t.e.input("\t");
    expect(t.last()).toBeNull();
    t.e.input("x");
    await flush();
    expect(t.last()).toBeNull();
  });

  it("accepts all or one word, and keeps the line in step", async () => {
    const t = setup();
    t.e.promptState(false);
    t.e.input("git c");
    await flush();
    expect(t.e.accept("word")).toBe("ommit");
    expect(t.last()).toBe(" -m fix");
    expect(t.e.accept("all")).toBe(" -m fix");
    expect(t.last()).toBeNull();
    expect(t.e.accept("all")).toBeNull(); // nothing left to take
  });

  it("remembers what you run, so this session's commands are suggested", () => {
    const t = setup();
    t.e.promptState(false);
    t.e.input("npm run dev\r");
    expect(t.recorded).toEqual(["npm run dev"]);
    t.e.input("git\t\r"); // unsure: not recorded
    expect(t.recorded).toEqual(["npm run dev"]);
  });

  it("stays out of the way when the shell suggests by itself, or when off", async () => {
    const t = setup();
    t.e.promptState(false);
    t.e.markShellSuggests();
    t.e.input("g");
    await flush();
    expect(t.shown.filter(Boolean)).toEqual([]);

    const off = setup();
    off.setEnabled(false);
    off.e.promptState(false);
    off.e.input("g");
    await flush();
    expect(off.shown.filter(Boolean)).toEqual([]);
  });

  it("ignores a history answer that arrives after the line moved on", async () => {
    const t = setup({ g: "git status", gx: "gx-tool" });
    t.e.promptState(false);
    t.e.input("g");
    t.e.input("\x7f");
    await flush();
    expect(t.last()).toBeNull();
  });
});

describe("offerFor", () => {
  it("offers the rest of a history command, drawn as is", () => {
    expect(offerFor("git st", { text: "git status", fix: false })).toEqual({
      draw: "atus",
      text: "atus",
      replace: false,
      fix: false,
    });
    expect(offerFor("ls", { text: "git status", fix: false })).toBeNull();
  });

  it("offers a correction on an empty line, and as a continuation", () => {
    expect(offerFor("", { text: "git push", fix: true })).toEqual({
      draw: "→ git push",
      text: "git push",
      replace: false,
      fix: true,
    });
    expect(offerFor("git p", { text: "git push", fix: true })?.text).toBe(
      "ush",
    );
  });

  it("offers a correction that doesn't continue the line as a replacement", () => {
    expect(offerFor("gti st", { text: "git status", fix: true })).toEqual({
      draw: "  → git status",
      text: "git status",
      replace: true,
      fix: true,
    });
    expect(
      offerFor("git status", { text: "git status", fix: true }),
    ).toBeNull();
  });
});

describe("command corrections", () => {
  it("reports each command's exit status once it ends", async () => {
    const t = setup();
    t.e.promptState(false);
    await t.run("git push", 0);
    await t.run("git psuh", 1);
    expect(t.finished).toEqual([
      ["git push", 0],
      ["git psuh", 1],
    ]);
  });

  it("offers the fix at the next prompt after a failure, and types it", async () => {
    const t = setup({}, { "git psuh": "git push" });
    t.e.promptState(false);
    await t.run("git psuh", 1);
    expect(t.last()).toBe("→ git push");
    expect(t.e.accept("all")).toBe("git push");
  });

  it("keeps the fix while you type it out, ahead of history", async () => {
    const t = setup({ g: "git status" }, { "git psuh": "git push" });
    t.e.promptState(false);
    await t.run("git psuh", 1);
    t.e.input("g");
    await flush();
    expect(t.last()).toBe("it push");
    t.e.input("x");
    await flush();
    expect(t.last()).toBeNull();
  });

  it("replaces a mistyped line with its learned fix, erasing it first", async () => {
    const t = setup({}, { "gti status": "git status" });
    t.e.promptState(false);
    t.e.input("gti st");
    await flush();
    expect(t.last()).toBe("  → git status");
    expect(t.e.accept("word")).toBe(`${"\x7f".repeat(6)}git status`);
  });

  it("takes the command's name from the shell when the line was edited out of sight", async () => {
    const t = setup();
    t.e.promptState(false);
    t.e.input("gi\t"); // Tab: the line is no longer known
    await t.run("", 127, "git sttaus");
    expect(t.finished).toEqual([["git sttaus", 127]]);
  });

  it("ignores a command name that wasn't asked for by an Enter", () => {
    const t = setup();
    t.e.promptState(false);
    t.e.commandStarted("rm -rf ~");
    t.e.commandFinished(1);
    expect(t.finished).toEqual([]);
  });

  it("never records or reports what's typed into a running program", async () => {
    const t = setup();
    t.e.promptState(false);
    t.e.input("sudo ls\r");
    t.e.promptState(true);
    t.e.commandStarted("");
    t.e.input("hunter2\r"); // the password prompt
    t.e.promptState(false);
    t.e.commandFinished(0);
    await flush();
    expect(t.recorded).toEqual(["sudo ls"]);
    expect(t.finished).toEqual([["sudo ls", 0]]);
  });

  it("drops a fix that arrives after you started typing", async () => {
    const t = setup({}, { "git psuh": "git push" });
    t.e.promptState(false);
    t.e.input("git psuh\r");
    t.e.promptState(true);
    t.e.promptState(false);
    t.e.commandFinished(1);
    t.e.input("l"); // typed before the answer came back
    await flush();
    expect(t.shown).not.toContain("→ git push");
  });

  it("with the shell's own suggestions on, offers only the fix on the empty line", async () => {
    const t = setup({ g: "git status" }, { "git psuh": "git push" });
    t.e.promptState(false);
    t.e.markShellSuggests();
    t.e.input("g");
    await flush();
    expect(t.shown.filter(Boolean)).toEqual([]); // history: the shell's job
    t.e.input("\x15");
    await t.run("git psuh", 1);
    expect(t.last()).toBe("→ git push");
    expect(t.e.accept("all")).toBe("git push");
    t.e.input("\x15"); // cleared instead of run
    await t.run("git psuh", 1);
    t.e.input("g"); // the shell's grey text takes over
    expect(t.last()).toBeNull();
    expect(t.finished.map(([c]) => c)).toEqual(["git psuh", "git psuh"]);
  });
});
