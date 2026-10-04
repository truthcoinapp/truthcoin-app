// Copy text to the clipboard: through the app (Rust writes the system clipboard), else the webview's own. True if it
// worked; when it didn't, the screen says so and the text stays selectable.
import { invoke } from "@tauri-apps/api/core";

export async function copy(text: string): Promise<boolean> {
  try {
    await invoke("copy_text", { text });
    return true;
  } catch {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      return false;
    }
  }
}

export const COPY_FAILED = "Couldn't copy: select the text and press Ctrl+C (⌘C on a Mac).";
