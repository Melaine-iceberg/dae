// The plain-C surface of the macOS platform readers.
//
// The accent is an AppKit object (`NSColor`), so the read lives in
// `macos_platform.mm`. Rust calls this C entry point rather than carrying an
// Objective-C dependency in the bridge.

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

#ifdef __cplusplus
}
#endif

#endif // DAE_QT_MACOS_PLATFORM_H
