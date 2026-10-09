import { IS_MAC } from "@/lib/platform";
import { invoke } from "@tauri-apps/api/core";

/**
 * TOSS Terminal shows no top bar or status bar unless asked: the window is
 * terminals only. The choice is a per-window convenience, kept in
 * localStorage like the sidebar's.
 */
const CHROME_SHOWN_KEY = "toss.chrome.shown";

export function readChromeShown(): boolean {
  try {
    return window.localStorage.getItem(CHROME_SHOWN_KEY) === "1";
  } catch {
    return false;
  }
}

export function saveChromeShown(shown: boolean): void {
  try {
    window.localStorage.setItem(CHROME_SHOWN_KEY, shown ? "1" : "0");
  } catch {}
}

/** macOS's window buttons go with the top bar, which leaves room for them. */
export function setWindowButtonsHidden(hidden: boolean): void {
  if (!IS_MAC) return;
  void invoke("window_set_buttons_hidden", { hidden }).catch(() => {});
}
