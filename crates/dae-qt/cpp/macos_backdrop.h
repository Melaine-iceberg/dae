// The macOS half of the window backdrop.
//
// The Qt view is an `NSView`; a sibling `NSVisualEffectView` inserted behind
// it blurs whatever is on the desktop. The implementation is Objective-C++
// (`macos_backdrop.mm`) so the header stays plain C for the bridge.

#pragma once

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Adds or removes the behind-window blur for the NSView owned by a window.
/// `dark` selects the vibrant appearance so the effect follows dae's theme
/// rather than only the system one. Returns false when the view has no window
/// (not attached yet).
bool dae_qt_macos_apply_blur(void *ns_view, bool enabled, bool dark);

#ifdef __cplusplus
}
#endif
