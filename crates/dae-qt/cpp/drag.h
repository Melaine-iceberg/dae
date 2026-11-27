// Native drag-out of local files and tabs.
//
// `QDrag` is the only piece of the OS drag interaction Qt exposes to C++, and
// the Qt shell's bridge has no binding for it. The functions live here rather
// than in Rust because the modal loop has to own the window's OLE apartment.

#pragma once

#include <QByteArray>
#include <QColor>
#include <QGuiApplication>
#include <QString>
#include <QStringList>
#include <QTimer>
#include <QUrl>

#include "rust/cxx.h"

namespace dae_qt {

/// Hands `paths` to the platform's drag loop, advertising `actions`.
///
/// `actions` is a bitmask of `Qt::CopyAction`, `Qt::MoveAction` and
/// `Qt::LinkAction`; the drop target decides which one it performs. `signal`
/// is the shell's signal colour, which draws the dashed frame around the
/// dragged icon. `count` and `verdict` are what §10.3 puts on the ghost: a
/// number badge when more than one entry is carried, and the predicted action
/// ("移动 3 项") spelled by the shell; `ink` and `canvas` are the two theme
/// colours those marks are drawn in, because the pixmap cannot read QML's
/// theme. The call blocks until the user drops or cancels, which is the
/// platform's modal loop: the window keeps painting while it runs. Returns
/// false when there is nothing to drag or no window to drag from.
bool dae_qt_start_file_drag(const QStringList& paths, int actions,
                            const QString& signal, int count,
                            const QString& verdict, const QString& ink,
                            const QString& canvas);

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
                          int height, const QString& background,
                          const QString& border, const QString& text);

/// Device-independent cursor position, the space QML `Window.x/y` uses.
///
/// The tab strip reads this once a native drag returns to place a torn-off
/// window at the release point. `QCursor::pos()` is cross-platform; keeping it
/// next to the drag means that path does not depend on a platform backend.
int dae_qt_cursor_logical_x();
int dae_qt_cursor_logical_y();

inline bool dae_qt_start_file_drag_rs(
    rust::Slice<const rust::String> paths, int actions, rust::Str signal,
    int count, rust::Str verdict, rust::Str ink, rust::Str canvas)
{
    QStringList list;
    list.reserve(static_cast<qsizetype>(paths.size()));
    for (const auto& path : paths) {
        list.append(QString::fromUtf8(path.data(), static_cast<qsizetype>(path.size())));
    }
    const QString qsignal = QString::fromUtf8(signal.data(), static_cast<qsizetype>(signal.size()));
    const QString qverdict = QString::fromUtf8(verdict.data(), static_cast<qsizetype>(verdict.size()));
    const QString qink = QString::fromUtf8(ink.data(), static_cast<qsizetype>(ink.size()));
    const QString qcanvas = QString::fromUtf8(canvas.data(), static_cast<qsizetype>(canvas.size()));
    // Run the blocking QDrag::exec() loop after the slot returns so QtBridge
    // releases its borrow before any nested frame or QThreadHandle callback runs.
    QTimer::singleShot(0, qApp, [list, actions, qsignal, count, qverdict, qink, qcanvas]() {
        dae_qt_start_file_drag(list, actions, qsignal, count, qverdict, qink, qcanvas);
    });
    return true;
}

inline int dae_qt_start_tab_drag_rs(
    rust::Str source_token, rust::Str title, int tab_index, int grab_x,
    int grab_y, int width, int height, rust::Str background,
    rust::Str border, rust::Str text)
{
    return dae_qt_start_tab_drag(
        QString::fromUtf8(source_token.data(), static_cast<qsizetype>(source_token.size())),
        QString::fromUtf8(title.data(), static_cast<qsizetype>(title.size())),
        tab_index, grab_x, grab_y, width, height,
        QString::fromUtf8(background.data(), static_cast<qsizetype>(background.size())),
        QString::fromUtf8(border.data(), static_cast<qsizetype>(border.size())),
        QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size())));
}

inline rust::Vec<rust::String> dae_qt_local_drop_paths(rust::Slice<const rust::String> urls)
{
    rust::Vec<rust::String> out;
    out.reserve(urls.size());
    for (const auto& raw : urls) {
        const QString qraw = QString::fromUtf8(raw.data(), static_cast<qsizetype>(raw.size()));
        const QUrl url = QUrl::fromUserInput(qraw);
        if (!url.isLocalFile()) {
            continue;
        }
        const QString local = url.toLocalFile();
        if (!local.isEmpty()) {
            const QByteArray utf8 = local.toUtf8();
            out.push_back(rust::String(utf8.constData(), static_cast<std::size_t>(utf8.size())));
        }
    }
    return out;
}

} // namespace dae_qt
