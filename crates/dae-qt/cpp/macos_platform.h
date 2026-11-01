// The plain-C surface of the macOS platform readers.
//
// Both values below are AppKit/Foundation objects (`NSColor`, `NSUserDefaults`),
// so the reads live in `macos_platform.mm`. Rust calls these C entry points
// rather than carrying an Objective-C dependency in the bridge.

#ifndef DAE_QT_MACOS_PLATFORM_H
#define DAE_QT_MACOS_PLATFORM_H

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Writes the current accent as `#rrggbb` into `buffer`.
///
/// `size` must be at least 8 bytes (seven characters and the NUL). Returns
/// false when the colour could not be resolved or the buffer is too small.
bool dae_qt_macos_accent_hex(char* buffer, int size);

/// What the user asked a title-bar double-click to do, from
/// `NSUserDefaults` key `AppleActionOnDoubleClick` (System Settings →
/// Desktop & Dock → "Double-click a window's title bar to").
///
/// Returns 0 for zoom ("Maximize"), 1 for minimize ("Minimize") and 2 for
/// "Do Nothing" ("None"). A missing or unrecognised value answers 0, which is
/// what macOS itself does for a mouse: the default is zoom.
int dae_qt_macos_titlebar_double_click(void);

#ifdef __cplusplus
}
#endif

#endif // DAE_QT_MACOS_PLATFORM_H
