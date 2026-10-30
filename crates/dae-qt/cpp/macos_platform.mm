#include "macos_platform.h"

#import <AppKit/AppKit.h>

#include <cstdio>

extern "C" bool dae_qt_macos_accent_hex(char* buffer, int size)
{
    @autoreleasepool {
        if (buffer == nullptr || size < 8)
            return false;

        // `controlAccentColor` is a dynamic colour: it means "the accent" until
        // it is asked for concrete components, and only answers those in a real
        // colour space. Without the conversion every component reads 0 and the
        // accent silently becomes black.
        NSColor* accent = [NSColor controlAccentColor];
        NSColor* rgb = [accent colorUsingColorSpace:[NSColorSpace sRGBColorSpace]];
        if (rgb == nil)
            return false;

        // The conversion above guarantees an RGB colour, so the component
        // getters are safe; the `getRed:...` primitive is avoided because its
        // BOOL return is ambiguous across SDK versions.
        const CGFloat red = rgb.redComponent;
        const CGFloat green = rgb.greenComponent;
        const CGFloat blue = rgb.blueComponent;

        const auto channel = [](CGFloat value) {
            const CGFloat clamped = value < 0.0 ? 0.0 : (value > 1.0 ? 1.0 : value);
            return static_cast<int>(clamped * 255.0 + 0.5);
        };
        std::snprintf(buffer, static_cast<size_t>(size), "#%02x%02x%02x",
                      channel(red), channel(green), channel(blue));
        return true;
    }
}
