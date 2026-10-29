// Behind-window blur for the Qt Quick shell.
//
// `QWindow::winId()` on macOS is the Qt view (`NSView*`), whose superview is
// the window's theme frame. The effect view is inserted as a sibling *below*
// the Qt view: a subview of the Qt view would be composited above its content,
// while a sibling in the theme frame sits behind the whole client area. The
// QML chrome is translucent, so the blur shows through exactly where the shell
// paints a tint.
//
// The map is weak-to-strong: when Qt destroys the view (a torn-off window),
// AppKit's zeroing weak key drops the entry and releases the effect view with
// it, so a recycled pointer can never resurrect an old blur.

#import <AppKit/AppKit.h>
#import <dispatch/dispatch.h>

static NSMapTable<NSView *, NSVisualEffectView *> *dae_effects;

extern "C" bool dae_qt_macos_apply_blur(void *native_view, bool enabled, bool dark)
{
    @autoreleasepool {
        NSView *view = (__bridge NSView *)native_view;
        if (!view || ![view isKindOfClass:[NSView class]])
            return false;

        static dispatch_once_t once;
        dispatch_once(&once, ^{
            dae_effects = [NSMapTable weakToStrongObjectsMapTable];
        });

        NSVisualEffectView *effect = [dae_effects objectForKey:view];
        if (!enabled) {
            if (effect) {
                [effect removeFromSuperview];
                [dae_effects removeObjectForKey:view];
            }
            return true;
        }

        NSWindow *window = view.window;
        NSView *host = view.superview;
        if (!window || !host)
            return false;

        if (!effect) {
            effect = [[NSVisualEffectView alloc] initWithFrame:view.frame];
            // Under-window blending samples the desktop behind the window,
            // which is the closest native analogue of Acrylic. The active-state
            // follow keeps inactive windows from looking permanently blurred.
            effect.material = NSVisualEffectMaterialUnderWindowBackground;
            effect.blendingMode = NSVisualEffectBlendingModeBehindWindow;
            effect.state = NSVisualEffectStateFollowsWindowActiveState;
            effect.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
            [host addSubview:effect positioned:NSWindowBelow relativeTo:view];
            [dae_effects setObject:effect forKey:view];
        }

        effect.appearance = [NSAppearance appearanceNamed:dark
            ? NSAppearanceNameVibrantDark
            : NSAppearanceNameVibrantLight];
        effect.frame = view.frame;

        // A translucent QQuickWindow needs a transparent window backing; the
        // default window background would paint over the effect view.
        window.opaque = NO;
        window.backgroundColor = [NSColor clearColor];
        return true;
    }
}
