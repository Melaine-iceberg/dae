// The macOS application menu.
//
// A QGuiApplication has no menu bar of its own: without one, Cmd+Q, Cmd+H,
// Cmd+M, About, Services and the Window menu simply do not exist, even though
// every native macOS app has them. This installs the standard menus with
// first-responder targets, so Qt's view keeps owning the actual key handling
// (undo/cut/copy/paste/selectAll) while AppKit owns the application commands.

#ifndef DAE_QT_MACOS_MENU_H
#define DAE_QT_MACOS_MENU_H

namespace dae_qt {

// Deliberately no Qt include: `macos_menu.mm` is compiled by a bare `cc`
// invocation alongside the AppKit code, which has no Qt include path.
#if defined(__APPLE__)
void dae_qt_install_macos_menu();
#else
inline void dae_qt_install_macos_menu() {}
#endif

} // namespace dae_qt

#endif // DAE_QT_MACOS_MENU_H
