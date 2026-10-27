// Native drag-out of local files.
//
// `QDrag` is the only piece of the OS drag interaction Qt exposes to C++, and
// the Qt shell's bridge has no binding for it. The function lives here rather
// than in Rust because the modal loop has to own the window's OLE apartment.

#pragma once

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

} // namespace dae_qt
