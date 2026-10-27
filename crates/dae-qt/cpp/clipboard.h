// The system clipboard's text.
//
// The Qt shell has no binding for `QClipboard`, and the file list the shell
// already writes is a different format: `CF_HDROP`/`text/uri-list` carries
// entries, "复制路径" carries one string that a chat window or a terminal is
// expected to paste. Both live on the same clipboard, so writing one replaces
// the other — which is exactly what Explorer does.

#pragma once

#include <QString>

namespace dae_qt {

/// Replaces the clipboard's text and, where the platform has a separate
/// selection buffer (X11), that too — a middle-click paste should show the
/// same path. An empty string clears the clipboard rather than leaving the
/// previous contents behind.
void dae_qt_set_clipboard_text(const QString& text);

} // namespace dae_qt
