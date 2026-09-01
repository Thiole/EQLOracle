// why: navigator.clipboard.writeText silently fails under WebKitGTK
// (verified empty clipboard on a real machine after a "copied" flow) --
// OS clipboard via the tauri plugin first, webview API as the plain-
// browser/mock fallback. Callers get an honest boolean, never a silent
// nothing-happened.
import { writeText } from '@tauri-apps/plugin-clipboard-manager';

export async function copyText(text: string): Promise<boolean> {
  try {
    await writeText(text);
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
