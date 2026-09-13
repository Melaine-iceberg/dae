import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { deleteSelection, getSelectedText } from "@/lib/text-editing";

// WebView2 的原生复制/剪切会更新剪贴板但不会被 Windows 剪贴板历史
// (Win+V)记录,因此全局拦截 copy/cut,统一改用插件原生写入。
// paste 保持原生:读取系统剪贴板本身没有问题。
export function setupNativeClipboardBridge(): void {
  document.addEventListener(
    "copy",
    (event) => {
      const text = getSelectedText(event.target);
      if (!text) return;

      event.preventDefault();
      void writeText(text).catch((error) => {
        console.warn("Unable to write text to clipboard", error);
      });
    },
    true,
  );

  document.addEventListener(
    "cut",
    (event) => {
      const text = getSelectedText(event.target);
      if (!text) return;

      event.preventDefault();
      void writeText(text).catch((error) => {
        console.warn("Unable to write text to clipboard", error);
      });
      deleteSelection(event.target);
    },
    true,
  );
}
