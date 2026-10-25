// Native window state for the Qt Quick shell (gates 3 and 4).
//
// The spike kept one `State` for "the" window and a second map bolted on for
// gate 4's torn-off window. That split is gone: every window is an entry in
// one HWND-keyed map, and the same code paths (style, backdrop, hit test,
// WM_NCCALCSIZE) run for the first window and the nth.
//
// What stays native, and why:
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

#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <windowsx.h>
#include <dwmapi.h>

#include <QAbstractNativeEventFilter>
#include <QCoreApplication>
#include <QDir>
#include <QGuiApplication>
#include <QHash>
#include <QPointer>
#include <QQuickWindow>
#include <QStandardPaths>
#include <QString>
#include <QTimer>
#include <QWindow>

#include "ime.h"
#include "lookup.h"

namespace dae_qt {

// Raw DWM attribute numbers. The SDK spells some of them under
// DWMWINDOWATTRIBUTE, but naming the values here keeps them next to the code
// that uses them and matches the Tauri module's `windows` crate.
constexpr DWORD kDwmwaUseImmersiveDarkMode = 20;
constexpr DWORD kDwmwaSystemBackdropType = 38;
constexpr DWORD kDwmwaMicaEffect = 1029; // undocumented, builds 22000-22522
constexpr int kDwmsbtNone = 1;
constexpr int kDwmsbtMainWindow = 2; // Mica
constexpr int kDwmsbtTransientWindow = 3; // Acrylic

// The QML title bar is 40 dp tall and each window button 46 dp wide. These are
// physical pixels on this side because WM_NCHITTEST speaks client pixels,
// which Qt does not scale for us in a native filter.
constexpr double kTitleBarDp = 40.0;
constexpr double kButtonDp = 46.0;

/// Everything the native half needs to know about one window.
///
/// `QPointer` is what makes a closed window safe: QML may destroy a torn-off
/// window at any time, and a queued maximize callback or a late hit test must
/// see a null window instead of a dangling pointer.
struct WindowState
{
    QPointer<QQuickWindow> window;
    HWND hwnd = nullptr;
    qreal dpr = 1.0;
    int material = 2; // 0 mica, 1 acrylic, 2 none
    bool dark = false;
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
/// title bar, so the layout is defined once (here) and QML draws to it.
inline LRESULT hit_test_for(HWND hwnd, qreal dpr, POINT screen)
{
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
            *result = hit_test_for(msg->hwnd, state->dpr, screen);
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

/// A translucent QQuickWindow needs its default alpha buffer switched on
/// before the first window is created. Called from `main` before the engine
/// loads QML.
inline void dae_qt_enable_alpha()
{
    QQuickWindow::setDefaultAlphaBuffer(true);
}

inline QString dae_qt_home_path()
{
    return QStandardPaths::writableLocation(QStandardPaths::HomeLocation);
}

} // namespace dae_qt
