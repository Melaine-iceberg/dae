// Native window state for the Qt Quick shell (gates 3 and 4).
//
// The spike kept one `State` for "the" window and a second map bolted on for
// gate 4's torn-off window. That split is gone: on Windows every window is an
// entry in one HWND-keyed map, and the same code paths (style, backdrop, hit
// test, WM_NCCALCSIZE) run for the first window and the nth.
//
// What stays native on Windows, and why:
//
//   * **The frame.** A frameless QQuickWindow has no native frame. The window
//     gets WS_THICKFRAME / WS_MAXIMIZEBOX back after creation, and
//     WM_NCHITTEST maps the QML title bar to the native hit codes. Returning
//     HTCAPTION and letting DefWindowProc run the move loop is what makes drag,
//     double-click maximize and Aero Snap native instead of reimplemented.
//     Returning HTMAXBUTTON on the maximize button is what opens Snap Layouts.
//   * **The backdrop.** Tauri reaches Mica/Acrylic through window-vibrancy; Qt
//     has no equivalent, so the DWM attributes are written directly. The
//     *decision* (which build, whether transparency is allowed) stays in Rust
//     (`src/material.rs`), which is also the Tauri module's rule.
//   * **The cancel guard.** See cpp/ime.h.
//
// Linux and macOS have neither a DWM nor a WM_NCHITTEST hook. The entry points
// below keep the same signatures there and do only the portable part (confirm
// the token names a window, force the platform window to exist); QML drives
// `startSystemMove` / `startSystemResize` instead of a caption hit test.
//
// Linux additionally implements Wayland's `ext-background-effect-v1`: the
// generated protocol code extends Qt's own `wl_surface`, and a compositor that
// does not advertise the global simply answers "no blur". macOS puts an
// `NSVisualEffectView` behind the Qt view instead (cpp/macos_backdrop.mm).

#pragma once

#include <QtGlobal>

#if defined(Q_OS_WIN)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <windowsx.h>
#include <dwmapi.h>

#include <QAbstractNativeEventFilter>
#include <QCoreApplication>
#include <QHash>
#include <QPointer>
#include <QTimer>
#endif

#include <QFont>
#include <QFontDatabase>
#include <QGuiApplication>
#include <QQuickWindow>
#include <QString>
#include <QStringList>
#include <QStyleHints>
#include <QWindow>

#include <cstdio>

#if defined(Q_OS_LINUX) && QT_CONFIG(wayland)
#include <wayland-client.h>

#include <cstring>

#include <QHash>
#include <qguiapplication_platform.h>
#include <qpa/qplatformnativeinterface.h>

#include "wayland/ext-background-effect-v1-client-protocol.h"
#endif

#include "lookup.h"

#if defined(Q_OS_MACOS)
#include "macos_backdrop.h"
#endif

namespace dae_qt {

#if defined(Q_OS_WIN)

// Raw DWM attribute numbers. The SDK spells some of them under
// DWMWINDOWATTRIBUTE, but naming the values here keeps them next to the code
// that uses them and matches the Tauri module's `windows` crate.
constexpr DWORD kDwmwaUseImmersiveDarkMode = 20;
constexpr DWORD kDwmwaWindowCornerPreference = 33;
constexpr DWORD kDwmwaSystemBackdropType = 38;
constexpr DWORD kDwmwaMicaEffect = 1029; // undocumented, builds 22000-22522
constexpr int kDwmsbtNone = 1;
constexpr int kDwmsbtMainWindow = 2; // Mica
constexpr int kDwmsbtTransientWindow = 3; // Acrylic
constexpr int kDwmcprRound = 2;

// The QML chrome bar is 44 dp tall and each window button 46 dp wide. These are
// physical pixels on this side because WM_NCHITTEST speaks client pixels,
// which Qt does not scale for us in a native filter.
//
// 44 is not cosmetic: the tab strip lives inside the bar now, so the bar has to
// be tall enough to hold a 34 dp tab with 5 dp above and below it. The height
// and `TitleBar.implicitHeight` are the same number and both are load-bearing.
constexpr double kTitleBarDp = 44.0;
constexpr double kButtonDp = 46.0;

/// Everything the native half needs to know about one window.
///
/// `QPointer` is what makes a closed window safe: QML may destroy a torn-off
/// window at any time, and a queued maximize callback or a late hit test must
/// see a null window instead of a dangling pointer.
struct WindowState
{
    QPointer<QQuickWindow> window;
    /// The QML `objectName`; what a tab drag names when it lands here.
    QString token;
    HWND hwnd = nullptr;
    qreal dpr = 1.0;
    int material = 2; // 0 mica, 1 acrylic, 2 none
    bool dark = false;
    /// The part of the bar that must stay a client area, in dp from the client's
    /// left edge: where the tabs and their `+` are. Everything else in the bar
    /// is caption, so the empty strip and the mark at the left both drag the
    /// window natively and keep Aero Snap and double-click maximise.
    ///
    /// QML owns these numbers because only QML knows how wide the tabs are; the
    /// right edge moves as tabs are opened and closed. `left == right` means
    /// "no client band", which is what a window with no tabs gets.
    double client_left_dp = 0.0;
    double client_right_dp = 0.0;
};

inline QHash<quintptr, WindowState>& window_states()
{
    static QHash<quintptr, WindowState> value;
    return value;
}

inline quintptr window_key(HWND hwnd)
{
    return reinterpret_cast<quintptr>(hwnd);
}

inline WindowState* state_for(HWND hwnd)
{
    if (!hwnd)
        return nullptr;
    auto it = window_states().find(window_key(hwnd));
    return it == window_states().end() ? nullptr : &it.value();
}

inline int titlebar_px_for(qreal dpr)
{
    return qRound(kTitleBarDp * dpr);
}

inline int button_px_for(qreal dpr)
{
    return qRound(kButtonDp * dpr);
}

/// The thickness Windows itself uses for a resize border at this DPI. This is
/// one visible pixel wider than the QML frame line.
inline int resize_border_px_for(HWND hwnd)
{
    const UINT dpi = hwnd ? GetDpiForWindow(hwnd) : 96;
    return GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi)
        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
}

/// The native hit test for the QML chrome. Screen coordinates in, one of the
/// HT* constants out.
///
/// Resize edges win over the caption because the QML frame is drawn inside
/// them; while maximized there is no resize edge, exactly as on a native
/// window. The button strip is the three rightmost `kButtonDp` columns of the
/// bar, so the layout is defined once (here) and QML draws to it. Between them
/// sits the tab band, which is the one region of the bar that has to reach
/// QML: a tab that hit-tested as caption could not be clicked, only dragged.
inline LRESULT hit_test_for(const WindowState& state, POINT screen)
{
    HWND hwnd = state.hwnd;
    const qreal dpr = state.dpr;
    RECT window_rect;
    GetWindowRect(hwnd, &window_rect);
    const bool zoomed = IsZoomed(hwnd) != FALSE;
    const int border = resize_border_px_for(hwnd);

    if (!zoomed) {
        const bool left = screen.x >= window_rect.left && screen.x < window_rect.left + border;
        const bool right = screen.x < window_rect.right && screen.x >= window_rect.right - border;
        const bool top = screen.y >= window_rect.top && screen.y < window_rect.top + border;
        const bool bottom = screen.y < window_rect.bottom && screen.y >= window_rect.bottom - border;
        if (top && left)
            return HTTOPLEFT;
        if (top && right)
            return HTTOPRIGHT;
        if (bottom && left)
            return HTBOTTOMLEFT;
        if (bottom && right)
            return HTBOTTOMRIGHT;
        if (left)
            return HTLEFT;
        if (right)
            return HTRIGHT;
        if (top)
            return HTTOP;
        if (bottom)
            return HTBOTTOM;
    }

    POINT client = screen;
    ScreenToClient(hwnd, &client);
    RECT client_rect;
    GetClientRect(hwnd, &client_rect);
    const int titlebar = titlebar_px_for(dpr);
    const int button = button_px_for(dpr);

    if (client.y >= 0 && client.y < titlebar) {
        const int right = client_rect.right;
        // Minimize and close are ordinary client areas: QML draws their hover
        // and owns their click. Maximize stays a native non-client zone at all
        // times, because HTMAXBUTTON is what opens Snap Layouts. Windows does
        // not re-run the hit test on press, so a pressed-state exception here
        // would never be seen; the NC click is handled on the down instead.
        if (client.x >= right - 2 * button && client.x < right - button)
            return HTMAXBUTTON;
        if (client.x >= right - 3 * button && client.x < right)
            return HTCLIENT;
        const int band_left = qRound(state.client_left_dp * dpr);
        const int band_right = qRound(state.client_right_dp * dpr);
        if (band_right > band_left && client.x >= band_left && client.x < band_right)
            return HTCLIENT;
        return HTCAPTION;
    }

    return HTCLIENT;
}

inline void disable_blur_behind(HWND hwnd)
{
    DWM_BLURBEHIND blur{};
    blur.dwFlags = DWM_BB_ENABLE;
    blur.fEnable = FALSE;
    DwmEnableBlurBehindWindow(hwnd, &blur);
}

/// An empty blur region makes DWM composite the desktop behind the client
/// area instead of the window's own black. This is the same call winit's
/// `with_transparent(true)` makes for the Tauri shell; Qt's
/// WA_TranslucentBackground alone leaves the D3D alpha path to the driver.
inline bool enable_blur_behind(HWND hwnd)
{
    HRGN region = CreateRectRgn(0, 0, -1, -1);
    DWM_BLURBEHIND blur{};
    blur.dwFlags = DWM_BB_ENABLE | DWM_BB_BLURREGION;
    blur.fEnable = TRUE;
    blur.hRgnBlur = region;
    blur.fTransitionOnMaximized = FALSE;
    const HRESULT result = DwmEnableBlurBehindWindow(hwnd, &blur);
    DeleteObject(region);
    return SUCCEEDED(result) != FALSE;
}

inline void style_window(HWND hwnd)
{
    LONG_PTR style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    // Qt created the window as WS_POPUP because of FramelessWindowHint. Put
    // back the pieces of a native overlapped window that give it shadow,
    // rounded corners, the Snap contracts and a size box, while the client
    // area still covers the whole window (Qt returns 0 from WM_NCCALCSIZE for
    // a frameless window).
    style &= ~LONG_PTR(WS_POPUP | WS_CHILD);
    style |= WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX
        | WS_CLIPSIBLINGS | WS_CLIPCHILDREN;
    SetWindowLongPtrW(hwnd, GWL_STYLE, style);
    SetWindowPos(hwnd, nullptr, 0, 0, 0, 0,
                 SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);

    // Windows 11 only rounds windows it recognises as standard frames. This
    // one answers WM_NCCALCSIZE with 0 (the QML client covers the whole
    // window), and it is translucent, so DWM leaves it square unless asked.
    // Unknown on Windows 10, where the call simply fails.
    int corner = kDwmcprRound;
    DwmSetWindowAttribute(hwnd, kDwmwaWindowCornerPreference, &corner, sizeof(corner));
}

inline bool apply_backdrop_to(HWND hwnd, int material, bool dark)
{
    if (!hwnd)
        return false;

    const BOOL dark_flag = dark ? TRUE : FALSE;
    DwmSetWindowAttribute(hwnd, kDwmwaUseImmersiveDarkMode, &dark_flag, sizeof(dark_flag));

    if (material == 2) { // none
        int none = kDwmsbtNone;
        DwmSetWindowAttribute(hwnd, kDwmwaSystemBackdropType, &none, sizeof(none));
        BOOL off = FALSE;
        DwmSetWindowAttribute(hwnd, kDwmwaMicaEffect, &off, sizeof(off));
        disable_blur_behind(hwnd);
        return true;
    }

    if (material == 3) // Wayland-only effect
        return false;

    enable_blur_behind(hwnd);

    const int type = material == 0 ? kDwmsbtMainWindow : kDwmsbtTransientWindow;
    const HRESULT backdrop =
        DwmSetWindowAttribute(hwnd, kDwmwaSystemBackdropType, &type, sizeof(type));

    if (FAILED(backdrop) && material == 0) {
        // The 22000-22522 band knows Mica only through the undocumented flag
        // the Tauri module's window-vibrancy version also falls back to.
        BOOL on = TRUE;
        return SUCCEEDED(DwmSetWindowAttribute(hwnd, kDwmwaMicaEffect, &on, sizeof(on)));
    }
    return SUCCEEDED(backdrop);
}

/// Qt's frameless path returns 0 for every size, which makes the client the
/// whole window rect. When maximized, Windows sizes a WS_THICKFRAME window to
/// the work area *plus* the invisible resize border (11 px at 150%), so that
/// client would be 22 px larger than the work area and run under the taskbar.
/// Handing DWM the work area as the client keeps the QML content aligned while
/// the extra window area stays off-screen.
inline bool fix_zoomed_client(HWND hwnd, WPARAM wParam, LPARAM lParam, LRESULT* result)
{
    if (!wParam || !IsZoomed(hwnd))
        return false;
    auto* params = reinterpret_cast<NCCALCSIZE_PARAMS*>(lParam);
    MONITORINFO monitor{};
    monitor.cbSize = sizeof(monitor);
    if (params
        && GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &monitor)) {
        params->rgrc[0] = monitor.rcWork;
        *result = 0;
        return true;
    }
    return false;
}

/// Acts on the maximize button once the physical button is up.
///
/// DefWindowProc's non-client button tracking owns the matching
/// WM_NCLBUTTONUP — the filter never sees it — and maximizing from inside that
/// tracking loop re-entered Qt and crashed. A poll waits for the release, then
/// lets the loop unwind before calling Qt's window-state API.
inline void maximize_after_release(QPointer<QQuickWindow> window)
{
    auto* timer = new QTimer(qApp);
    timer->setInterval(20);
    QObject::connect(timer, &QTimer::timeout, qApp, [timer, window]() {
        if ((GetAsyncKeyState(VK_LBUTTON) & 0x8000) != 0)
            return;
        timer->stop();
        timer->deleteLater();
        QTimer::singleShot(80, qApp, [window]() {
            if (!window)
                return;
            if (IsZoomed(reinterpret_cast<HWND>(window->winId())))
                window->showNormal();
            else
                window->showMaximized();
        });
    });
    timer->start();
}

class NativeFilter final : public QAbstractNativeEventFilter
{
public:
    bool nativeEventFilter(const QByteArray& type, void* message, qintptr* result) override
    {
        if (type != "windows_generic_MSG" && type != "windows_dispatcher_MSG")
            return false;
        auto* msg = static_cast<MSG*>(message);
        if (!msg)
            return false;

        WindowState* state = state_for(msg->hwnd);
        if (!state)
            return false;

        switch (msg->message) {
        case WM_NCHITTEST: {
            const POINT screen{ GET_X_LPARAM(msg->lParam), GET_Y_LPARAM(msg->lParam) };
            *result = hit_test_for(*state, screen);
            return true;
        }
        case WM_NCLBUTTONDOWN:
            if (msg->wParam == HTMAXBUTTON)
                maximize_after_release(state->window);
            return false;
        case WM_NCCALCSIZE:
            return fix_zoomed_client(msg->hwnd, msg->wParam, msg->lParam, result);
        case WM_DPICHANGED:
            if (state->window)
                state->dpr = state->window->devicePixelRatio();
            return false;
        default:
            return false;
        }
    }
};

inline void install_filter_once()
{
    static bool installed = false;
    if (installed)
        return;
    QGuiApplication::instance()->installNativeEventFilter(new NativeFilter());
    installed = true;
}

/// Registers the window named `token` and applies the native frame.
///
/// The QML side creates the window hidden, calls this, and only then shows it.
/// `winId()` forces the platform window to exist, so the style and the
/// backdrop are in place before the first frame. Attaching twice is a no-op
/// apart from refreshing the DPR.
inline bool dae_qt_attach(const QString& token)
{
    QQuickWindow* window = find_window_by_token(token);
    if (!window)
        return false;

    install_filter_once();

    const HWND hwnd = reinterpret_cast<HWND>(window->winId());
    if (!hwnd)
        return false;

    const quintptr key = window_key(hwnd);
    auto existing = window_states().find(key);
    if (existing != window_states().end()) {
        existing->window = window;
        existing->dpr = window->devicePixelRatio();
        return true;
    }

    WindowState state;
    state.window = window;
    state.token = token;
    state.hwnd = hwnd;
    state.dpr = window->devicePixelRatio();
    window_states().insert(key, state);

    style_window(hwnd);

    // The HWND key must go away with the window: Windows recycles handles, and
    // a stale entry would hit-test a new window with the old DPR and material.
    QObject::connect(window, &QObject::destroyed, qApp, [key](QObject*) {
        window_states().remove(key);
    });
    return true;
}

inline bool dae_qt_apply_backdrop(const QString& token, int material, bool dark)
{
    QQuickWindow* window = find_window_by_token(token);
    if (!window)
        return false;
    HWND hwnd = reinterpret_cast<HWND>(window->winId());
    WindowState* state = state_for(hwnd);
    if (!state)
        return false;

    state->material = material;
    state->dark = dark;
    return apply_backdrop_to(hwnd, material, dark);
}

/// Records which horizontal slice of the 44 dp bar must hit-test as client, in
/// dp from the client's left edge. Called by the tab strip whenever the number
/// or the width of the tabs changes.
inline bool dae_qt_set_caption_client(const QString& token, double left, double right)
{
    QQuickWindow* window = find_window_by_token(token);
    if (!window)
        return false;
    WindowState* state = state_for(reinterpret_cast<HWND>(window->winId()));
    if (!state)
        return false;
    state->client_left_dp = left;
    state->client_right_dp = right;
    return true;
}

/// Windows composites Mica/Acrylic itself; the generic blur capability is the
/// Wayland/macOS path and has nothing to advertise here.
inline bool dae_qt_blur_supported()
{
    return false;
}

#else // !Q_OS_WIN

#if defined(Q_OS_LINUX) && QT_CONFIG(wayland)

// Wayland background blur through `ext-background-effect-v1`. Qt does not
// expose the effect, but it owns the `wl_display` and the `wl_surface` this
// code has to extend: the public native interface hands over both, and the
// generated protocol bindings (wayland-scanner 1.26) are compiled into this
// file. On X11 there is no such surface and the effect is unavailable.

/// The extension is bound once per process: the registry and the manager do
/// not belong to a window, and binding a second manager for every window would
/// just add roundtrips.
struct WaylandBackgroundEffect
{
    wl_display* display = nullptr;
    wl_compositor* compositor = nullptr;
    ext_background_effect_manager_v1* manager = nullptr;
    bool blur = false;
    bool initialized = false;

    bool supported() const { return manager && blur; }
};

struct WaylandRegistryState
{
    WaylandBackgroundEffect* effect;
};

inline void wayland_global(void* data, wl_registry* registry, uint32_t name,
                           const char* interface, uint32_t version)
{
    auto* state = static_cast<WaylandRegistryState*>(data);
    if (std::strcmp(interface, ext_background_effect_manager_v1_interface.name) != 0)
        return;
    if (state->effect->manager)
        return;
    state->effect->manager = static_cast<ext_background_effect_manager_v1*>(
        wl_registry_bind(registry, name, &ext_background_effect_manager_v1_interface,
                         qMin(version, 1u)));
}

inline void wayland_global_remove(void*, wl_registry*, uint32_t) {}

/// The capability is the compositor's answer to "can you blur". It is sent
/// when the global is bound and again whenever it changes, so the listener
/// has to stay alive for the lifetime of the manager.
inline void wayland_capabilities(void* data, ext_background_effect_manager_v1*, uint32_t flags)
{
    static_cast<WaylandBackgroundEffect*>(data)->blur =
        (flags & EXT_BACKGROUND_EFFECT_MANAGER_V1_CAPABILITY_BLUR) != 0;
}

inline WaylandBackgroundEffect& wayland_background_effect()
{
    static WaylandBackgroundEffect effect;
    if (effect.initialized)
        return effect;
    effect.initialized = true;

    // `wayland-egl` was the Qt 5 name; Qt 6 reports plain "wayland" for every
    // Wayland integration, but the prefix keeps this from missing a variant.
    if (!QGuiApplication::platformName().startsWith(QStringLiteral("wayland")))
        return effect;

    // `nativeInterface()` is a member, so it needs an instance; the macro is
    // the QGuiApplication cast of the one `QCoreApplication::instance()`.
    auto* app = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>();
    if (!app)
        return effect;
    effect.display = app->display();
    effect.compositor = app->compositor();
    if (!effect.display || !effect.compositor)
        return effect;

    WaylandRegistryState state{ &effect };
    static const wl_registry_listener registry_listener{ wayland_global,
                                                         wayland_global_remove };
    static const ext_background_effect_manager_v1_listener manager_listener{
        wayland_capabilities
    };

    // A private queue means the blocking roundtrips below dispatch only these
    // bindings. Qt's own queue keeps whatever it already had pending, which is
    // the same split `linux_graphics.rs` used for its startup probe.
    wl_event_queue* queue = wl_display_create_queue(effect.display);
    wl_registry* registry = wl_display_get_registry(effect.display);
    if (queue && registry) {
        wl_proxy_set_queue(reinterpret_cast<wl_proxy*>(registry), queue);
        wl_registry_add_listener(registry, &registry_listener, &state);
        wl_display_roundtrip_queue(effect.display, queue);

        if (effect.manager) {
            wl_proxy_set_queue(reinterpret_cast<wl_proxy*>(effect.manager), queue);
            ext_background_effect_manager_v1_add_listener(effect.manager, &manager_listener,
                                                          &effect);
            // The capabilities event is sent on bind; one more roundtrip
            // settles it before the first window asks for a material.
            wl_display_roundtrip_queue(effect.display, queue);
            // Capability changes are rare but not one-shot; they have to reach
            // the listener through Qt's own dispatch once this queue is gone.
            wl_proxy_set_queue(reinterpret_cast<wl_proxy*>(effect.manager), nullptr);
        }
    }
    if (registry)
        wl_registry_destroy(registry);
    if (queue)
        wl_event_queue_destroy(queue);
    return effect;
}

struct WaylandBlurSurface
{
    ext_background_effect_surface_v1* effect = nullptr;
};

inline QHash<quintptr, WaylandBlurSurface>& wayland_blur_surfaces()
{
    static QHash<quintptr, WaylandBlurSurface> surfaces;
    return surfaces;
}

inline wl_surface* wayland_surface_for(QQuickWindow* window)
{
    auto* native = QGuiApplication::platformNativeInterface();
    if (!native)
        return nullptr;
    return static_cast<wl_surface*>(native->nativeResourceForWindow("surface", window));
}

inline void update_blur_region(QQuickWindow* window)
{
    auto& surfaces = wayland_blur_surfaces();
    auto it = surfaces.find(reinterpret_cast<quintptr>(window));
    if (it == surfaces.end() || !it->effect)
        return;
    auto& effect = wayland_background_effect();
    if (!effect.compositor)
        return;

    wl_region* region = wl_compositor_create_region(effect.compositor);
    if (!region)
        return;
    // Surface-local coordinates are logical pixels, which is what QWindow
    // reports. The compositor clips the region to the surface, so a resize
    // only needs a fresh region at the new size.
    wl_region_add(region, 0, 0, qMax(1, window->width()), qMax(1, window->height()));
    ext_background_effect_surface_v1_set_blur_region(it->effect, region);
    wl_region_destroy(region);
}

inline bool set_wayland_blur(QQuickWindow* window, bool enabled)
{
    auto& effect = wayland_background_effect();

    const quintptr key = reinterpret_cast<quintptr>(window);
    auto& surfaces = wayland_blur_surfaces();
    auto it = surfaces.find(key);

    if (!enabled) {
        if (it != surfaces.end() && it->effect) {
            // Destroying the object removes the effect on the next commit;
            // there is no need to send a null region first.
            ext_background_effect_surface_v1_destroy(it->effect);
            it->effect = nullptr;
            // Both the destroy and a new region are double-buffered, so a
            // quiet window needs one frame to show either change.
            window->update();
        }
        return true;
    }

    if (!effect.supported())
        return false;

    if (it == surfaces.end() || !it->effect) {
        wl_surface* surface = wayland_surface_for(window);
        if (!surface)
            return false;
        auto* proxy =
            ext_background_effect_manager_v1_get_background_effect(effect.manager, surface);
        if (!proxy)
            return false;

        if (it == surfaces.end()) {
            surfaces.insert(key, WaylandBlurSurface{ proxy });
            // The key is the window address, which the allocator may reuse. It
            // has to go away with the window so a torn-off one never inherits
            // the previous effect.
            QObject::connect(window, &QObject::destroyed, qApp, [key](QObject*) {
                auto& surfaces = wayland_blur_surfaces();
                auto it = surfaces.find(key);
                if (it == surfaces.end())
                    return;
                if (it->effect)
                    ext_background_effect_surface_v1_destroy(it->effect);
                surfaces.erase(it);
            });
            QObject::connect(window, &QWindow::widthChanged, window,
                             [window](int) { update_blur_region(window); });
            QObject::connect(window, &QWindow::heightChanged, window,
                             [window](int) { update_blur_region(window); });
        } else {
            it->effect = proxy;
        }
    }

    update_blur_region(window);
    // The region is double-buffered state. Without a commit the compositor
    // would not see it until the window happens to redraw, which a settings
    // change with no animation might never trigger.
    window->update();
    return true;
}

inline bool dae_qt_blur_supported()
{
    return wayland_background_effect().supported();
}

inline bool dae_qt_apply_blur(const QString& token, bool enabled, bool)
{
    QQuickWindow* window = find_window_by_token(token);
    if (!window)
        return false;
    return set_wayland_blur(window, enabled);
}

#elif defined(Q_OS_MACOS)

/// `NSVisualEffectView` exists on every macOS release Qt 6 supports, so the
/// only question is whether the user wants it.
inline bool dae_qt_blur_supported()
{
    return true;
}

inline bool dae_qt_apply_blur(const QString& token, bool enabled, bool dark)
{
    QQuickWindow* window = find_window_by_token(token);
    if (!window)
        return false;
    // On macOS Qt's WId is the window's NSView.
    return ::dae_qt_macos_apply_blur(reinterpret_cast<void*>(window->winId()), enabled, dark);
}

#else // neither Wayland nor macOS

inline bool dae_qt_blur_supported()
{
    return false;
}

inline bool dae_qt_apply_blur(const QString&, bool, bool)
{
    return false;
}

#endif // platform blur support

// There is no native frame to style and no hit test to feed. Keep the contract
// the Rust bridge and QML rely on: false means "no such window", true means the
// request was accepted. Attach still forces the platform window to exist so the
// hidden-window ordering in QML (`attach`, then show) is the same everywhere.
inline bool dae_qt_attach(const QString& token)
{
    QQuickWindow* window = find_window_by_token(token);
    if (!window)
        return false;
    window->winId();
    return true;
}

/// Windows draws Mica/Acrylic; Wayland and macOS may draw a background blur.
/// Everything else keeps the solid chrome.
inline bool dae_qt_apply_backdrop(const QString& token, int material, bool dark)
{
    QQuickWindow* window = find_window_by_token(token);
    if (!window)
        return false;
    if (material == 3) // blur
        return dae_qt_apply_blur(token, true, dark);
    if (material == 2) { // none
        dae_qt_apply_blur(token, false, dark);
        return true;
    }
    return false;
}

/// The caption band is a Windows hit-test input. QML's drag and resize
/// handlers are the portable equivalent, so this is accepted and ignored.
inline bool dae_qt_set_caption_client(const QString& token, double, double)
{
    return find_window_by_token(token) != nullptr;
}

#endif // Q_OS_WIN

/// A translucent QQuickWindow needs its default alpha buffer switched on
/// before the first window is created. Called from `main` before the engine
/// loads QML.
inline void dae_qt_enable_alpha()
{
    QQuickWindow::setDefaultAlphaBuffer(true);
}

/// The shell's UI face.
///
/// Qt asks the platform theme for an application font. On Windows that answer
/// is still the legacy "Segoe UI" whatever the display language, while the OS
/// paints its own UI with the face it chose for that language: Microsoft YaHei
/// UI on Simplified Chinese Windows, JhengHei UI on Traditional Chinese, Yu
/// Gothic UI on Japanese, Malgun Gothic on Korean. A QFont family list is a
/// per-glyph fallback list, so the language face has to come first: with Segoe
/// first, one label that mixes Latin and Chinese would paint the Latin from
/// Segoe and the Chinese from YaHei, which is the mixed face this is meant to
/// avoid.
///
/// The non-client message font is not enough on its own: Windows 11 reports
/// plain "Segoe UI" there even on a Chinese system, so the UI language id is
/// the authoritative source. A message font the user has actually changed is
/// still honoured first. A Latin display language keeps the Windows 11
/// Variable Text cut (/ Segoe UI on Windows 10).
///
/// macOS and Linux already default to their platform UI face (SF Pro, the
/// fontconfig default), so this is a no-op there rather than a second opinion
/// about which family is "the system font".
inline void dae_qt_apply_system_font()
{
#if defined(Q_OS_WIN)
    QFont font = QGuiApplication::font();

    // The face Windows' own UI uses for the current display language. The
    // Traditional Chinese variants share the language id but not the face. The
    // non-UI names follow as a safety net for installs that ship only the
    // older family name.
    QStringList preferred;
    const LANGID language = GetUserDefaultUILanguage();
    switch (PRIMARYLANGID(language)) {
    case LANG_CHINESE:
        switch (SUBLANGID(language)) {
        case SUBLANG_CHINESE_SIMPLIFIED:
        case SUBLANG_CHINESE_SINGAPORE:
            preferred << QStringLiteral("Microsoft YaHei UI")
                      << QStringLiteral("Microsoft YaHei");
            break;
        default:
            preferred << QStringLiteral("Microsoft JhengHei UI")
                      << QStringLiteral("Microsoft JhengHei");
            break;
        }
        break;
    case LANG_JAPANESE:
        preferred << QStringLiteral("Yu Gothic UI")
                  << QStringLiteral("Yu Gothic");
        break;
    case LANG_KOREAN:
        preferred << QStringLiteral("Malgun Gothic");
        break;
    default:
        break;
    }

    // The message face is what the user's own desktop uses. On Windows 11 it
    // is still "Segoe UI" even on a Chinese system, so only a customised or
    // non-Latin face is worth putting ahead of the language face; plain
    // "Segoe UI" is just the legacy Latin default.
    QString messageFace;
    NONCLIENTMETRICSW metrics{};
    metrics.cbSize = sizeof(metrics);
    if (SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, sizeof(metrics),
                              &metrics, 0)) {
        messageFace = QString::fromWCharArray(metrics.lfMessageFont.lfFaceName);
    }
    const bool customMessageFace = !messageFace.isEmpty()
        && messageFace.compare(QStringLiteral("Segoe UI"),
                               Qt::CaseInsensitive) != 0;

    const QStringList installed = QFontDatabase::families();
    const QString latinFace = installed.contains(
            QStringLiteral("Segoe UI Variable Text"))
        ? QStringLiteral("Segoe UI Variable Text")
        : QStringLiteral("Segoe UI");

    QStringList families;
    if (customMessageFace && installed.contains(messageFace))
        families << messageFace;
    for (const QString& face : preferred) {
        if (installed.contains(face) && !families.contains(face))
            families << face;
    }
    families << latinFace;

    font.setFamilies(families);
    QGuiApplication::setFont(font);
#endif
}

/// The OS light/dark preference. Qt reads the platform's setting, so the
/// Mica/Acrylic tint on Windows agrees with the rest of the desktop instead of
/// being fixed at dark; the same query is what the QML theme follows
/// everywhere else.
inline bool dae_qt_system_dark()
{
    return QGuiApplication::styleHints()->colorScheme() == Qt::ColorScheme::Dark;
}

// ---------------------------------------------------------------------------
// Window focus.
//
// A native drag runs in the platform's own loop (`drag.cpp`), so the strip no
// longer polls the pointer, the window under it or the physical button.
// Everything the tab drag needs lives in drag.cpp and is cross-platform.
// ---------------------------------------------------------------------------

inline bool dae_qt_focus_window(const QString& token)
{
    QQuickWindow* window = find_window_by_token(token);
    if (!window)
        return false;

    if (!window->isVisible())
        window->show();

#if defined(Q_OS_WIN)
    const auto hwnd = reinterpret_cast<HWND>(window->winId());
    // SW_RESTORE also brings back the placement a minimized window had before
    // (maximized often), where showNormal() would always shrink it.
    if (hwnd && IsIconic(hwnd))
        ShowWindow(hwnd, SW_RESTORE);
#endif
    // Qt maps these to the platform's own raise/activate calls. On macOS a
    // background application still needs an explicit app activation before a
    // window can come forward; `raise()`/`requestActivate()` alone leave the
    // previous app in front.
#if defined(Q_OS_MACOS)
    ::dae_qt_macos_activate_app();
#endif
    window->raise();
    window->requestActivate();
#if defined(Q_OS_WIN)
    if (hwnd)
        SetForegroundWindow(hwnd);
#endif
    return true;
}

/// Sends Qt's own diagnostics to stderr.
///
/// A QML component that fails to instantiate reports why only through the
/// engine's warnings, and a GUI-subsystem process has no console for the
/// default handler to find. Installing this before the engine loads turns a
/// silent blank window into the exact file, line and reason.
inline void dae_qt_install_message_handler()
{
    static bool installed = false;
    if (installed)
        return;
    installed = true;
    qInstallMessageHandler([](QtMsgType, const QMessageLogContext&, const QString& message) {
        const QByteArray utf8 = message.toUtf8();
        std::fprintf(stderr, "qt: %s\n", utf8.constData());
        std::fflush(stderr);
    });
}

} // namespace dae_qt
