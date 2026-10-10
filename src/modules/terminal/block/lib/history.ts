import { invoke } from "@tauri-apps/api/core";

/**
 * A grey suggestion: the rest of a command from history, or (`fix`) the
 * correction of a command that failed before.
 */
export type Suggestion = { text: string; fix: boolean };

export function historySuggest(line: string): Promise<Suggestion | null> {
  return invoke<Suggestion | null>("history_suggest", { line }).catch(
    () => null,
  );
}

export function historyCommands(prefix: string, limit = 50): Promise<string[]> {
  return invoke<string[]>("history_commands", { prefix, limit }).catch(() => []);
}

export function historyList(query: string, limit = 200): Promise<string[]> {
  return invoke<string[]>("history_list", { query, limit }).catch(() => []);
}

export function historyRecord(command: string): void {
  void invoke("history_record", { command }).catch(() => {});
}

/**
 * A command finished: learn from whether it worked. For a failure, pass the
 * end of its output; the answer is a correction to offer, or null.
 */
export function historyFinish(
  pane: number,
  command: string,
  exit: number | null,
  output: string,
): Promise<string | null> {
  return invoke<string | null>("history_finish", {
    pane,
    command,
    exit,
    output,
  }).catch(() => null);
}

/** Whether an exit status is a failure worth correcting (not a Ctrl+C). */
export function isFailure(exit: number | null): boolean {
  return exit !== null && exit > 0 && exit < 128;
}
