import { IS_MAC } from "@/lib/platform";
import { invoke } from "@tauri-apps/api/core";

/**
 * Whether the top and status bars are shown (they are unless hidden with the
 * view.zenMode shortcut). A per-window convenience, kept in localStorage like
 * the sidebar's.
 */
const CHROME_HIDDEN_KEY = "toss.chrome.hidden";

export function readChromeShown(): boolean {
  try {
    return window.localStorage.getItem(CHROME_HIDDEN_KEY) !== "1";
  } catch {
    return true;
  }
}

export function saveChromeShown(shown: boolean): void {
  try {
    window.localStorage.setItem(CHROME_HIDDEN_KEY, shown ? "0" : "1");
  } catch {}
}

/** macOS's window buttons go with the top bar, which leaves room for them. */
export function setWindowButtonsHidden(hidden: boolean): void {
  if (!IS_MAC) return;
  void invoke("window_set_buttons_hidden", { hidden }).catch(() => {});
}
