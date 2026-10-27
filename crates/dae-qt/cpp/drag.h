// Native drag-out of local files and tabs.
//
// `QDrag` is the only piece of the OS drag interaction Qt exposes to C++, and
// the Qt shell's bridge has no binding for it. The functions live here rather
// than in Rust because the modal loop has to own the window's OLE apartment.

#pragma once

#include <QColor>
#include <QString>
#include <QStringList>

namespace dae_qt {

/// Hands `paths` to the platform's drag loop, advertising `actions`.
///
/// `actions` is a bitmask of `Qt::CopyAction`, `Qt::MoveAction` and
/// `Qt::LinkAction`; the drop target decides which one it performs. The call
/// blocks until the user drops or cancels, which is the platform's modal loop:
/// the window keeps painting while it runs. Returns false when there is
/// nothing to drag or no window to drag from.
bool dae_qt_start_file_drag(const QStringList& paths, int actions);

/// The MIME type a dragged tab carries. QML's window-level `DropArea` matches
/// on this exact string; `application/x-dae-tab` never leaves this process.
constexpr char kTabDragMimeType[] = "application/x-dae-tab";

/// Hands a tab gesture to the platform's drag loop.
///
/// The drag image is a chip painted from the tab's own title and colors, and
/// `grab_x`/`grab_y` — the press point inside the tab, in logical pixels —
/// becomes the pixmap's hot spot. The platform then keeps that point under the
/// cursor, so the preview follows the grab point instead of being centered on
/// it, and the same contract holds for OLE, NSDraggingSession and X11/Wayland
/// DnD. The call blocks until the drop lands or the gesture is cancelled and
/// returns the resulting `Qt::DropAction`; `Qt::IgnoreAction` means no drop
/// target took it (the desktop, another app, or Escape).
int dae_qt_start_tab_drag(const QString& source_token, const QString& title,
                          int tab_index, int grab_x, int grab_y, int width,
                          int height, const QColor& background,
                          const QColor& border, const QColor& text);

/// Device-independent cursor position, the space QML `Window.x/y` uses.
///
/// The tab strip reads this once a native drag returns to place a torn-off
/// window at the release point. `QCursor::pos()` is cross-platform; keeping it
/// next to the drag means that path does not depend on a platform backend.
int dae_qt_cursor_logical_x();
int dae_qt_cursor_logical_y();

} // namespace dae_qt
