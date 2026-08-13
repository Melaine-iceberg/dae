import { writeText } from "@tauri-apps/plugin-clipboard-manager";

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

function getSelectedText(target: EventTarget | null): string {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    return target.value.slice(target.selectionStart ?? 0, target.selectionEnd ?? 0);
  }

  return window.getSelection()?.toString() ?? "";
}

function deleteSelection(target: EventTarget | null): void {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    const start = target.selectionStart ?? 0;
    const end = target.selectionEnd ?? 0;
    if (start === end) return;

    const prototype =
      target instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(
      target,
      target.value.slice(0, start) + target.value.slice(end),
    );
    target.dispatchEvent(new Event("input", { bubbles: true }));
    target.setSelectionRange(start, start);
    return;
  }

  if (target instanceof HTMLElement && target.isContentEditable) {
    document.execCommand("delete");
  }
}
