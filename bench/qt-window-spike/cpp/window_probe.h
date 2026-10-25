// Native half of gate 3 of docs/decisions/engine-qt-quick.md §5.
//
// The spike asks three things of the Windows side, and all three are answered
// here rather than in QML:
//
//   * **The backdrop.** The x86-tauri shell reaches Mica/Acrylic through
//     `window-vibrancy`; Qt has no equivalent, so this file writes the same
//     DWM attributes directly. The *decision* (which build, whether the user
//     allows transparency) is still made in Rust
//     (`src/material.rs`, ported from `src-tauri/src/window_material/windows.rs`);
//     this file only applies the answer and reports what DWM accepted.
//   * **The chrome.** A frameless QQuickWindow has no native frame, so the
//     window gets WS_THICKFRAME / WS_MAXIMIZEBOX back after creation and a
//     WM_NCHITTEST answer that maps the QML title bar and its three buttons to
//     the native hit codes. Returning HTMAXBUTTON is also what makes Windows
//     11 offer the Snap Layouts flyout; returning HTCAPTION and letting
//     DefWindowProc run the move loop is what makes drag, double-click and
//     Aero Snap native instead of reimplemented.
//   * **The evidence.** Real SendInput mouse gestures drive the scenarios,
//     every WM_NCHITTEST / WM_NCLBUTTONDOWN answer is counted, and a screen
//     capture loop samples the window while the DWM tint switches. QML owns
//     the script; this file owns the clock, the input and the pixels.
//
// Header-only, like the probes in the other two spikes: cxx-qt compiles the
// generated translation unit and calls these functions.

#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <windowsx.h>
#include <dwmapi.h>

#include <QAbstractNativeEventFilter>
#include <QDir>
#include <QElapsedTimer>
#include <QGuiApplication>
#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickWindow>
#include <QTimer>
#include <QString>
#include <QVector>
#include <QWindow>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <memory>
#include <string>

namespace dae_window_probe {

// Raw DWM attribute numbers. The SDK installed here has some of them under
// DWMWINDOWATTRIBUTE, but spelling them out keeps the values visible next to
// the code that uses them and matches the Tauri module's `windows` crate.
constexpr DWORD kDwmwaUseImmersiveDarkMode = 20;
constexpr DWORD kDwmwaSystemBackdropType = 38;
constexpr DWORD kDwmwaMicaEffect = 1029; // undocumented, builds 22000–22522
constexpr int kDwmsbtNone = 1;
constexpr int kDwmsbtMainWindow = 2; // Mica
constexpr int kDwmsbtTransientWindow = 3; // Acrylic

// The caption buttons run their state change from a zero-delay timer, outside
// the native filter: ShowWindow sends WM_SIZE synchronously, and doing that
// inside QWindowsContext's dispatch re-enters it.

// The QML title bar is 40 dp tall and each window button 46 dp wide. They are
// physical pixels on this side because WM_NCHITTEST speaks client pixels,
// which Qt does not scale for us in a native filter.
constexpr double kTitleBarDp = 40.0;
constexpr double kButtonDp = 46.0;

struct HitRecord
{
    qint64 at_ms = 0;
    int code = 0;
    int screen_x = 0;
    int screen_y = 0;
    int client_x = 0;
    int client_y = 0;
    bool zoomed = false;
};

struct Sample
{
    qint64 at_ms = 0;
    QString tag;
    double mean = 0.0;
    double variance = 0.0;
    double p05 = 0.0;
    double p95 = 0.0;
    qint64 frames = 0;
    QString path;
};

struct State
{
    bool attached = false;
    bool connected = false;
    bool styled = false;
    QElapsedTimer clock;
    QQuickWindow* window = nullptr;
    HWND hwnd = nullptr;
    qreal dpr = 1.0;

    // Last backdrop request, for the result JSON.
    int material = 2; // 0 mica, 1 acrylic, 2 none
    bool dark = false;
    long dark_result = 0;
    long backdrop_result = 0;
    long mica_result = 0;
    bool blur_behind = false;
    bool style_applied = false;

    qint64 hit_counts[32] = {};
    qint64 nc_down_counts[32] = {};
    qint64 nc_up = 0;
    qint64 nc_dblclk = 0;
    qint64 nccalcsize = 0;
    qint64 entersizemove = 0;
    qint64 exitsizemove = 0;
    qint64 moving = 0;
    qint64 sizing = 0;
    qint64 erasebkgnd = 0;
    qint64 syscommand = 0;
    qint64 activate = 0;
    qint64 dpichanged = 0;
    qint64 frames = 0;
    qint64 before_rendering = 0;
    qint64 chrome_actions = 0;
    QVector<HitRecord> recent_hits;

    int sample_x = 0;
    int sample_y = 0;
    int sample_w = 0;
    int sample_h = 0;
    QVector<Sample> samples;

    int flyout_x = 0;
    int flyout_y = 0;
    int flyout_w = 0;
    int flyout_h = 0;
    QImage flyout_baseline;
};

inline State& state()
{
    static State value;
    return value;
}

/// One torn-off window. Gate 3 only ever attached to the focused window; gate 4
/// creates a second QQuickWindow and has to prove that the same native path
/// (style, backdrop, hit test, message loop) runs per HWND, not once per
/// process. The source window stays in `state()`; every additional window gets
/// an entry here, keyed by HWND.
struct SecondaryState
{
    QQuickWindow* window = nullptr;
    HWND hwnd = nullptr;
    qreal dpr = 1.0;

    int material = 2; // 0 mica, 1 acrylic, 2 none
    bool dark = false;
    long dark_result = 0;
    long backdrop_result = 0;
    long mica_result = 0;
    bool blur_behind = false;
    bool style_applied = false;

    qint64 hit_counts[32] = {};
    qint64 nc_down_counts[32] = {};
    qint64 nc_up = 0;
    qint64 nc_dblclk = 0;
    qint64 nccalcsize = 0;
    qint64 entersizemove = 0;
    qint64 exitsizemove = 0;
    qint64 moving = 0;
    qint64 sizing = 0;
    qint64 erasebkgnd = 0;
    qint64 activate = 0;
    qint64 dpichanged = 0;
    qint64 frames = 0;
    qint64 before_rendering = 0;
    qint64 chrome_actions = 0;
    QVector<HitRecord> recent_hits;
};

inline QHash<quintptr, SecondaryState>& secondary_states()
{
    static QHash<quintptr, SecondaryState> value;
    return value;
}

inline quintptr& current_secondary_key()
{
    static quintptr value = 0;
    return value;
}

inline quintptr window_key(HWND hwnd)
{
    return reinterpret_cast<quintptr>(hwnd);
}

inline SecondaryState* secondary_for(HWND hwnd)
{
    if (!hwnd)
        return nullptr;
    auto it = secondary_states().find(window_key(hwnd));
    return it == secondary_states().end() ? nullptr : &it.value();
}

inline SecondaryState* current_secondary()
{
    return secondary_for(reinterpret_cast<HWND>(current_secondary_key()));
}

inline qint64 now_ms()
{
    return state().clock.isValid() ? state().clock.elapsed() : 0;
}

inline QQuickWindow* quick_window()
{
    if (auto* focused = qobject_cast<QQuickWindow*>(QGuiApplication::focusWindow()))
        return focused;
    for (QWindow* candidate : QGuiApplication::topLevelWindows()) {
        if (auto* quick = qobject_cast<QQuickWindow*>(candidate))
            return quick;
    }
    return nullptr;
}

inline HWND window_handle()
{
    QQuickWindow* window = quick_window();
    return window ? reinterpret_cast<HWND>(window->winId()) : nullptr;
}

inline int titlebar_px_for(qreal dpr)
{
    return qRound(kTitleBarDp * dpr);
}

inline int button_px_for(qreal dpr)
{
    return qRound(kButtonDp * dpr);
}

inline int titlebar_px()
{
    return titlebar_px_for(state().dpr);
}

inline int button_px()
{
    return button_px_for(state().dpr);
}

/// The thickness Windows itself uses for a resize border at this DPI. This is
/// what Qt's `getResizeBorderThickness` uses for the expanded-client-area
/// path, and it is one visible pixel wider than the QML frame line.
inline int resize_border_px_for(HWND hwnd)
{
    const UINT dpi = hwnd ? GetDpiForWindow(hwnd) : 96;
    return GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi)
        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
}

inline int resize_border_px()
{
    return resize_border_px_for(state().hwnd);
}

inline QJsonArray point_json(POINT point)
{
    return QJsonArray{ int(point.x), int(point.y) };
}

inline QJsonArray rect_json(const RECT& rect)
{
    return QJsonArray{ int(rect.left), int(rect.top), int(rect.right), int(rect.bottom) };
}

inline std::unique_ptr<std::string> to_json(const QJsonObject& root)
{
    const QByteArray bytes = QJsonDocument(root).toJson(QJsonDocument::Compact);
    return std::make_unique<std::string>(bytes.constData(), size_t(bytes.size()));
}

inline std::unique_ptr<std::string> to_json(const QJsonArray& root)
{
    const QByteArray bytes = QJsonDocument(root).toJson(QJsonDocument::Compact);
    return std::make_unique<std::string>(bytes.constData(), size_t(bytes.size()));
}

inline POINT client_origin(HWND hwnd)
{
    POINT origin{ 0, 0 };
    ClientToScreen(hwnd, &origin);
    return origin;
}

/// The native hit test for the QML chrome. Screen coordinates in, one of the
/// HT* constants out — the same function is used by the live message filter
/// and by the invokable that probes it during the scripted run.
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

inline LRESULT hit_test(HWND hwnd, POINT screen)
{
    return hit_test_for(hwnd, state().dpr, screen);
}

inline bool maximize_native(bool via_qt)
{
    State& value = state();
    if (!value.hwnd || !value.window)
        return false;
    if (via_qt) {
        if (IsZoomed(value.hwnd))
            value.window->showNormal();
        else
            value.window->showMaximized();
    } else {
        ShowWindow(value.hwnd, IsZoomed(value.hwnd) ? SW_RESTORE : SW_MAXIMIZE);
    }
    return true;
}

inline void perform_maximize_toggle(bool was_zoomed)
{
    State& value = state();
    if (!value.window || !value.hwnd)
        return;
    if ((IsZoomed(value.hwnd) != FALSE) != was_zoomed) {
        // DefWindowProc performed the same toggle; do not undo it.
        return;
    }
    if (was_zoomed)
        value.window->showNormal();
    else
        value.window->showMaximized();
    value.chrome_actions += 1;
}

/// Acts on the maximize button once the physical button is up.
///
/// DefWindowProc's non-client button tracking owns the matching
/// WM_NCLBUTTONUP — the filter never sees it — and maximizing from inside
/// that tracking loop re-entered Qt and crashed. A poll waits for the release,
/// then lets the loop unwind before calling Qt's window-state API.
inline void maximize_after_release(bool was_zoomed)
{
    auto* timer = new QTimer(qApp);
    timer->setInterval(20);
    QObject::connect(timer, &QTimer::timeout, qApp, [timer, was_zoomed]() {
        if ((GetAsyncKeyState(VK_LBUTTON) & 0x8000) != 0)
            return;
        timer->stop();
        timer->deleteLater();
        QTimer::singleShot(80, qApp, [was_zoomed]() {
            perform_maximize_toggle(was_zoomed);
        });
    });
    timer->start();
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

inline void record_hit(SecondaryState& value, HWND hwnd, POINT screen, LRESULT code)
{
    POINT client = screen;
    ScreenToClient(hwnd, &client);
    HitRecord record;
    record.at_ms = now_ms();
    record.code = int(code);
    record.screen_x = screen.x;
    record.screen_y = screen.y;
    record.client_x = client.x;
    record.client_y = client.y;
    record.zoomed = IsZoomed(hwnd) != FALSE;
    value.recent_hits.push_back(record);
    if (value.recent_hits.size() > 240)
        value.recent_hits.remove(0, 60);
    if (code >= 0 && code < 32)
        value.hit_counts[code] += 1;
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

        State& value = state();
        if (value.hwnd && msg->hwnd == value.hwnd) {
            switch (msg->message) {
            case WM_NCHITTEST: {
                const POINT screen{ GET_X_LPARAM(msg->lParam), GET_Y_LPARAM(msg->lParam) };
                const LRESULT code = hit_test(msg->hwnd, screen);

                POINT client = screen;
                ScreenToClient(msg->hwnd, &client);
                HitRecord record;
                record.at_ms = now_ms();
                record.code = int(code);
                record.screen_x = screen.x;
                record.screen_y = screen.y;
                record.client_x = client.x;
                record.client_y = client.y;
                record.zoomed = IsZoomed(msg->hwnd) != FALSE;
                value.recent_hits.push_back(record);
                if (value.recent_hits.size() > 240)
                    value.recent_hits.remove(0, 60);
                if (code >= 0 && code < 32)
                    value.hit_counts[code] += 1;

                *result = code;
                return true;
            }
            case WM_NCLBUTTONDOWN:
                if (msg->wParam >= 0 && msg->wParam < 32)
                    value.nc_down_counts[msg->wParam] += 1;
                if (msg->wParam == HTMAXBUTTON)
                    maximize_after_release(IsZoomed(msg->hwnd) != FALSE);
                return false;
            case WM_NCLBUTTONUP:
                value.nc_up += 1;
                return false;
            case WM_NCLBUTTONDBLCLK:
                value.nc_dblclk += 1;
                return false;
            case WM_NCCALCSIZE:
                value.nccalcsize += 1;
                if (fix_zoomed_client(msg->hwnd, msg->wParam, msg->lParam, result))
                    return true;
                return false;
            case WM_ENTERSIZEMOVE:
                value.entersizemove += 1;
                return false;
            case WM_EXITSIZEMOVE:
                value.exitsizemove += 1;
                return false;
            case WM_MOVING:
                value.moving += 1;
                return false;
            case WM_SIZING:
                value.sizing += 1;
                return false;
            case WM_ERASEBKGND:
                value.erasebkgnd += 1;
                return false;
            case WM_ACTIVATE:
                value.activate += 1;
                return false;
            case WM_DPICHANGED:
                value.dpichanged += 1;
                return false;
            default:
                return false;
            }
        }

        SecondaryState* secondary = secondary_for(msg->hwnd);
        if (!secondary)
            return false;

        switch (msg->message) {
        case WM_NCHITTEST: {
            const POINT screen{ GET_X_LPARAM(msg->lParam), GET_Y_LPARAM(msg->lParam) };
            const LRESULT code = hit_test_for(msg->hwnd, secondary->dpr, screen);
            record_hit(*secondary, msg->hwnd, screen, code);
            *result = code;
            return true;
        }
        case WM_NCLBUTTONDOWN:
            if (msg->wParam >= 0 && msg->wParam < 32)
                secondary->nc_down_counts[msg->wParam] += 1;
            return false;
        case WM_NCLBUTTONUP:
            secondary->nc_up += 1;
            return false;
        case WM_NCLBUTTONDBLCLK:
            secondary->nc_dblclk += 1;
            return false;
        case WM_NCCALCSIZE:
            secondary->nccalcsize += 1;
            return fix_zoomed_client(msg->hwnd, msg->wParam, msg->lParam, result);
        case WM_ENTERSIZEMOVE:
            secondary->entersizemove += 1;
            return false;
        case WM_EXITSIZEMOVE:
            secondary->exitsizemove += 1;
            return false;
        case WM_MOVING:
            secondary->moving += 1;
            return false;
        case WM_SIZING:
            secondary->sizing += 1;
            return false;
        case WM_ERASEBKGND:
            secondary->erasebkgnd += 1;
            return false;
        case WM_ACTIVATE:
            secondary->activate += 1;
            return false;
        case WM_DPICHANGED:
            secondary->dpichanged += 1;
            return false;
        default:
            return false;
        }
    }
};

/// A translucent QQuickWindow needs its default alpha buffer switched on
/// before the first window is created. Called from `main` before the engine
/// loads QML, not from `attach`.
inline void enable_alpha()
{
    QQuickWindow::setDefaultAlphaBuffer(true);
}

/// The x86-tauri shell gets this from winit's `with_transparent(true)`: an
/// empty blur region makes DWM composite the desktop behind the client area
/// instead of the window's own black. DwmSetWindowAttribute(38) then replaces
/// the blur with the real material. Qt's WA_TranslucentBackground alone makes
/// the window layered but leaves D3D's alpha path to the driver; the same raw
/// call Tauri relies on is the one measured here.
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

inline void disable_blur_behind(HWND hwnd)
{
    DWM_BLURBEHIND blur{};
    blur.dwFlags = DWM_BB_ENABLE;
    blur.fEnable = FALSE;
    DwmEnableBlurBehindWindow(hwnd, &blur);
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

inline bool attach()
{
    State& value = state();
    if (!value.clock.isValid())
        value.clock.start();
    if (!value.attached) {
        QGuiApplication::instance()->installNativeEventFilter(new NativeFilter());
        value.attached = true;
    }

    QQuickWindow* window = quick_window();
    if (!window)
        return false;
    const HWND hwnd = reinterpret_cast<HWND>(window->winId());
    if (!hwnd)
        return false;

    value.window = window;
    value.hwnd = hwnd;
    value.dpr = window->devicePixelRatio();

    if (!value.styled) {
        style_window(hwnd);
        value.style_applied = true;
        value.styled = true;
    }

    if (!value.connected) {
        QObject::connect(window, &QQuickWindow::frameSwapped, window, []() { state().frames += 1; });
        QObject::connect(window, &QQuickWindow::beforeRendering, window,
                         []() { state().before_rendering += 1; });
        value.connected = true;
    }
    return true;
}

/// Attach the native half to a second QQuickWindow, matched by its title.
///
/// The QML side creates the window hidden with a per-cycle title, calls this,
/// and only then sets `visible`. `winId()` forces the platform window to exist,
/// so the frameless style and the backdrop are in place before the first
/// frame — the same order a shell needs if it does not want a visible flash of
/// the system frame. Frames are counted per HWND; the process-wide probe
/// callbacks in gate 3 would otherwise attribute a second window's paints to
/// the source window.
inline bool attach_secondary(const QString& title)
{
    QQuickWindow* window = nullptr;
    for (QWindow* candidate : QGuiApplication::topLevelWindows()) {
        auto* quick = qobject_cast<QQuickWindow*>(candidate);
        if (quick && quick->title() == title) {
            window = quick;
            break;
        }
    }
    if (!window)
        return false;

    const HWND hwnd = reinterpret_cast<HWND>(window->winId());
    if (!hwnd)
        return false;

    SecondaryState record;
    record.window = window;
    record.hwnd = hwnd;
    record.dpr = window->devicePixelRatio();
    style_window(hwnd);
    record.style_applied = true;

    const quintptr key = window_key(hwnd);
    secondary_states().insert(key, record);
    current_secondary_key() = key;

    QObject::connect(window, &QQuickWindow::frameSwapped, window, [key]() {
        auto it = secondary_states().find(key);
        if (it != secondary_states().end())
            it.value().frames += 1;
    });
    QObject::connect(window, &QQuickWindow::beforeRendering, window, [key]() {
        auto it = secondary_states().find(key);
        if (it != secondary_states().end())
            it.value().before_rendering += 1;
    });
    return true;
}

/// Drop the record after QML destroyed the window. Keeping it would make the
/// next cycle's HWND key collide with a recycled handle.
inline void forget_secondary()
{
    auto it = secondary_states().find(current_secondary_key());
    if (it != secondary_states().end())
        secondary_states().erase(it);
    current_secondary_key() = 0;
}

// ---------------------------------------------------------------------------
// Backdrop

inline bool apply_backdrop(int material, bool dark)
{
    State& value = state();
    HWND hwnd = value.hwnd;
    if (!hwnd)
        return false;

    value.material = material;
    value.dark = dark;

    const BOOL dark_flag = dark ? TRUE : FALSE;
    const HRESULT dark_result =
        DwmSetWindowAttribute(hwnd, kDwmwaUseImmersiveDarkMode, &dark_flag, sizeof(dark_flag));
    value.dark_result = long(dark_result);

    if (material == 2) { // none
        int none = kDwmsbtNone;
        const HRESULT backdrop =
            DwmSetWindowAttribute(hwnd, kDwmwaSystemBackdropType, &none, sizeof(none));
        value.backdrop_result = long(backdrop);
        BOOL off = FALSE;
        value.mica_result = long(DwmSetWindowAttribute(hwnd, kDwmwaMicaEffect, &off, sizeof(off)));
        disable_blur_behind(hwnd);
        value.blur_behind = false;
        return true;
    }

    value.blur_behind = enable_blur_behind(hwnd);

    const int type = material == 0 ? kDwmsbtMainWindow : kDwmsbtTransientWindow;
    HRESULT backdrop = DwmSetWindowAttribute(hwnd, kDwmwaSystemBackdropType, &type, sizeof(type));
    value.backdrop_result = long(backdrop);

    if (FAILED(backdrop) && material == 0) {
        // The 22000–22522 band knows Mica only through the undocumented flag
        // the Tauri module's window-vibrancy version also falls back to.
        BOOL on = TRUE;
        const HRESULT mica = DwmSetWindowAttribute(hwnd, kDwmwaMicaEffect, &on, sizeof(on));
        value.mica_result = long(mica);
        return SUCCEEDED(mica);
    }
    value.mica_result = long(S_OK);
    return SUCCEEDED(backdrop);
}

inline bool apply_secondary_backdrop(int material, bool dark)
{
    SecondaryState* secondary = current_secondary();
    if (!secondary || !secondary->hwnd)
        return false;
    const HWND hwnd = secondary->hwnd;

    secondary->material = material;
    secondary->dark = dark;

    const BOOL dark_flag = dark ? TRUE : FALSE;
    secondary->dark_result =
        long(DwmSetWindowAttribute(hwnd, kDwmwaUseImmersiveDarkMode, &dark_flag, sizeof(dark_flag)));

    if (material == 2) { // none
        int none = kDwmsbtNone;
        secondary->backdrop_result =
            long(DwmSetWindowAttribute(hwnd, kDwmwaSystemBackdropType, &none, sizeof(none)));
        BOOL off = FALSE;
        secondary->mica_result =
            long(DwmSetWindowAttribute(hwnd, kDwmwaMicaEffect, &off, sizeof(off)));
        disable_blur_behind(hwnd);
        secondary->blur_behind = false;
        return true;
    }

    secondary->blur_behind = enable_blur_behind(hwnd);

    const int type = material == 0 ? kDwmsbtMainWindow : kDwmsbtTransientWindow;
    const HRESULT backdrop =
        DwmSetWindowAttribute(hwnd, kDwmwaSystemBackdropType, &type, sizeof(type));
    secondary->backdrop_result = long(backdrop);

    if (FAILED(backdrop) && material == 0) {
        BOOL on = TRUE;
        secondary->mica_result =
            long(DwmSetWindowAttribute(hwnd, kDwmwaMicaEffect, &on, sizeof(on)));
        return SUCCEEDED(secondary->mica_result);
    }
    secondary->mica_result = long(S_OK);
    return SUCCEEDED(backdrop);
}

inline QJsonObject backdrop_state_for(HWND hwnd, int material, bool dark,
                                      long dark_result, long backdrop_result,
                                      long mica_result, bool blur_behind)
{
    QJsonObject root;
    root["material"] = material;
    root["dark"] = dark;
    root["dark_hr"] = QString::number(dark_result, 16);
    root["backdrop_hr"] = QString::number(backdrop_result, 16);
    root["mica_hr"] = QString::number(mica_result, 16);
    root["blur_behind"] = blur_behind;

    if (hwnd) {
        BOOL read_dark = FALSE;
        const HRESULT dark_hr =
            DwmGetWindowAttribute(hwnd, kDwmwaUseImmersiveDarkMode, &read_dark, sizeof(read_dark));
        root["read_dark"] = SUCCEEDED(dark_hr) ? bool(read_dark) : false;
        root["read_dark_hr"] = QString::number(dark_hr, 16);

        int type = -1;
        const HRESULT type_hr =
            DwmGetWindowAttribute(hwnd, kDwmwaSystemBackdropType, &type, sizeof(type));
        root["read_backdrop"] = SUCCEEDED(type_hr) ? type : -1;
        root["read_backdrop_hr"] = QString::number(type_hr, 16);

        const LONG_PTR exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        root["layered"] = (exstyle & WS_EX_LAYERED) != 0;

        BOOL composition = FALSE;
        DwmIsCompositionEnabled(&composition);
        root["composition"] = composition != FALSE;
    }
    return root;
}

inline QJsonObject backdrop_state_object()
{
    State& value = state();
    return backdrop_state_for(value.hwnd, value.material, value.dark, value.dark_result,
                              value.backdrop_result, value.mica_result, value.blur_behind);
}

// ---------------------------------------------------------------------------
// Geometry

inline QJsonObject chrome_regions_for(HWND hwnd, qreal dpr)
{
    QJsonObject root;
    if (!hwnd)
        return root;

    RECT client;
    GetClientRect(hwnd, &client);
    const POINT origin = client_origin(hwnd);
    const int titlebar = titlebar_px_for(dpr);
    const int button = button_px_for(dpr);
    const int border = resize_border_px_for(hwnd);
    const int width = client.right;
    const int height = client.bottom;

    const auto at = [&](int x, int y) {
        return QJsonArray{ int(origin.x) + x, int(origin.y) + y };
    };

    root["caption"] = at(width / 2 - 3 * button / 2, titlebar / 2);
    root["min"] = at(width - 3 * button + button / 2, titlebar / 2);
    root["max"] = at(width - 2 * button + button / 2, titlebar / 2);
    root["close"] = at(width - button + button / 2, titlebar / 2);
    root["content"] = at(width / 2, titlebar + (height - titlebar) / 2);
    root["border_left"] = at(std::max(2, border / 2), height / 2);
    root["border_right"] = at(width - std::max(2, border / 2), height / 2);
    root["border_top"] = at(width / 2, std::max(2, border / 2));
    root["border_bottom"] = at(width / 2, height - std::max(2, border / 2));
    root["corner_tl"] = at(std::max(2, border / 2), std::max(2, border / 2));
    root["corner_br"] = at(width - std::max(2, border / 2), height - std::max(2, border / 2));
    root["titlebar_px"] = titlebar;
    root["button_px"] = button;
    root["border_px"] = border;
    root["client_width_px"] = width;
    root["client_height_px"] = height;
    return root;
}

inline QJsonObject chrome_regions_object()
{
    State& value = state();
    return chrome_regions_for(value.hwnd, value.dpr);
}

inline int hit_test_at(int x, int y)
{
    State& value = state();
    if (!value.hwnd)
        return -1;
    const LPARAM point = MAKELPARAM(x, y);
    return int(SendMessageW(value.hwnd, WM_NCHITTEST, 0, point));
}

inline QJsonObject window_info_for(HWND hwnd, qreal dpr, int material, bool dark,
                                   long dark_result, long backdrop_result, long mica_result,
                                   bool blur_behind)
{
    QJsonObject root;
    if (!hwnd)
        return root;

    RECT window_rect;
    GetWindowRect(hwnd, &window_rect);
    RECT frame_rect = window_rect;
    DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &frame_rect,
                          sizeof(frame_rect));
    RECT client_rect;
    GetClientRect(hwnd, &client_rect);
    const POINT origin = client_origin(hwnd);

    MONITORINFO monitor{};
    monitor.cbSize = sizeof(monitor);
    const HMONITOR handle = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    GetMonitorInfoW(handle, &monitor);

    wchar_t class_name[256] = {};
    GetClassNameW(hwnd, class_name, 256);
    wchar_t title[512] = {};
    GetWindowTextW(hwnd, title, 512);

    root["hwnd"] = QString::number(reinterpret_cast<quintptr>(hwnd), 16);
    root["title"] = QString::fromWCharArray(title);
    root["class"] = QString::fromWCharArray(class_name);
    root["qt_version"] = QString::fromLatin1(qVersion());
    root["os"] = QSysInfo::prettyProductName();
    root["dpr"] = dpr;
    root["window_rect"] = rect_json(window_rect);
    root["frame_rect"] = rect_json(frame_rect);
    root["client_rect"] = rect_json(client_rect);
    root["client_origin"] = point_json(origin);
    root["work_area"] = rect_json(monitor.rcWork);
    root["monitor"] = rect_json(monitor.rcMonitor);
    root["zoomed"] = IsZoomed(hwnd) != FALSE;
    root["iconic"] = IsIconic(hwnd) != FALSE;
    root["visible"] = IsWindowVisible(hwnd) != FALSE;
    root["foreground"] = GetForegroundWindow() == hwnd;
    root["style"] = QString::number(GetWindowLongPtrW(hwnd, GWL_STYLE), 16);
    root["exstyle"] = QString::number(GetWindowLongPtrW(hwnd, GWL_EXSTYLE), 16);
    root["titlebar_px"] = titlebar_px_for(dpr);
    root["button_px"] = button_px_for(dpr);
    root["border_px"] = resize_border_px_for(hwnd);
    root["backdrop"] = backdrop_state_for(hwnd, material, dark, dark_result, backdrop_result,
                                          mica_result, blur_behind);
    return root;
}

inline QJsonObject window_info_object()
{
    State& value = state();
    return window_info_for(value.hwnd, value.dpr, value.material, value.dark,
                           value.dark_result, value.backdrop_result, value.mica_result,
                           value.blur_behind);
}

// ---------------------------------------------------------------------------
// Message counters

inline QJsonObject counter_json(const qint64 (&counts)[32])
{
    QJsonObject root;
    for (int code = 0; code < 32; ++code) {
        if (counts[code])
            root[QString::number(code)] = double(counts[code]);
    }
    return root;
}

template <typename T>
inline QJsonObject chrome_stats_for(const T& value)
{
    QJsonObject root;
    root["hit_test"] = counter_json(value.hit_counts);
    root["nc_down"] = counter_json(value.nc_down_counts);
    root["nc_up"] = double(value.nc_up);
    root["nc_dblclk"] = double(value.nc_dblclk);
    root["nccalcsize"] = double(value.nccalcsize);
    root["entersizemove"] = double(value.entersizemove);
    root["exitsizemove"] = double(value.exitsizemove);
    root["moving"] = double(value.moving);
    root["sizing"] = double(value.sizing);
    root["erasebkgnd"] = double(value.erasebkgnd);
    root["activate"] = double(value.activate);
    root["dpichanged"] = double(value.dpichanged);
    root["frames"] = double(value.frames);
    root["before_rendering"] = double(value.before_rendering);
    root["chrome_actions"] = double(value.chrome_actions);

    QJsonArray recent;
    const qsizetype first = std::max<qsizetype>(0, value.recent_hits.size() - 40);
    for (qsizetype index = first; index < value.recent_hits.size(); ++index) {
        const HitRecord& hit = value.recent_hits[index];
        QJsonObject entry;
        entry["t_ms"] = double(hit.at_ms);
        entry["code"] = hit.code;
        entry["screen"] = QJsonArray{ hit.screen_x, hit.screen_y };
        entry["client"] = QJsonArray{ hit.client_x, hit.client_y };
        entry["zoomed"] = hit.zoomed;
        recent.append(entry);
    }
    root["recent"] = recent;
    return root;
}

inline QJsonObject chrome_stats_object()
{
    return chrome_stats_for(state());
}

template <typename T>
inline void clear_stats_for(T& value)
{
    std::fill(std::begin(value.hit_counts), std::end(value.hit_counts), 0);
    std::fill(std::begin(value.nc_down_counts), std::end(value.nc_down_counts), 0);
    value.nc_up = 0;
    value.nc_dblclk = 0;
    value.nccalcsize = 0;
    value.entersizemove = 0;
    value.exitsizemove = 0;
    value.moving = 0;
    value.sizing = 0;
    value.erasebkgnd = 0;
    value.activate = 0;
    value.dpichanged = 0;
    value.recent_hits.clear();
}

inline void clear_chrome_stats()
{
    clear_stats_for(state());
}

inline void clear_secondary_stats()
{
    if (auto* secondary = current_secondary())
        clear_stats_for(*secondary);
}

// ---------------------------------------------------------------------------
// Mouse input (real SendInput, guarded by "our window is foreground")

inline bool is_foreground_for(HWND hwnd)
{
    return hwnd && GetForegroundWindow() == hwnd;
}

inline bool is_foreground()
{
    return is_foreground_for(state().hwnd);
}

inline bool set_foreground_for(HWND hwnd)
{
    if (!hwnd)
        return false;
    if (GetForegroundWindow() == hwnd)
        return true;
    if (!IsWindowVisible(hwnd))
        ShowWindow(hwnd, SW_SHOW);

    const HWND foreground = GetForegroundWindow();
    const DWORD foreground_thread =
        foreground ? GetWindowThreadProcessId(foreground, nullptr) : 0;
    const DWORD this_thread = GetCurrentThreadId();
    const bool attached = foreground_thread && foreground_thread != this_thread
        && AttachThreadInput(foreground_thread, this_thread, TRUE) != FALSE;

    // Windows' foreground lock refuses a plain SetForegroundWindow from a
    // process launched by a terminal. A synthetic Alt tap is the documented
    // "the user did something" escape hatch; without it the scripted run got
    // no drag at all when the terminal happened to own the foreground.
    keybd_event(VK_MENU, 0, 0, 0);
    BringWindowToTop(hwnd);
    SetForegroundWindow(hwnd);
    SetActiveWindow(hwnd);
    keybd_event(VK_MENU, 0, KEYEVENTF_KEYUP, 0);
    SwitchToThisWindow(hwnd, TRUE);
    SetFocus(hwnd);

    if (attached)
        AttachThreadInput(foreground_thread, this_thread, FALSE);
    return GetForegroundWindow() == hwnd;
}

inline bool set_foreground()
{
    return set_foreground_for(state().hwnd);
}

inline bool set_secondary_foreground()
{
    SecondaryState* secondary = current_secondary();
    return secondary && set_foreground_for(secondary->hwnd);
}

inline bool is_secondary_foreground()
{
    SecondaryState* secondary = current_secondary();
    return secondary && is_foreground_for(secondary->hwnd);
}

inline bool send_mouse(DWORD flags, int x, int y, bool absolute)
{
    INPUT input{};
    input.type = INPUT_MOUSE;
    input.mi.dwFlags = flags;
    if (absolute) {
        const int vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        const int vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        const int vw = std::max(1, GetSystemMetrics(SM_CXVIRTUALSCREEN));
        const int vh = std::max(1, GetSystemMetrics(SM_CYVIRTUALSCREEN));
        input.mi.dx = int(std::lround(double(x - vx) * 65535.0 / double(vw - 1)));
        input.mi.dy = int(std::lround(double(y - vy) * 65535.0 / double(vh - 1)));
        input.mi.dwFlags |= MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    }
    return SendInput(1, &input, sizeof(INPUT)) == 1;
}

inline bool move_cursor(int x, int y)
{
    return send_mouse(MOUSEEVENTF_MOVE, x, y, true);
}

inline bool mouse_down_for(HWND hwnd)
{
    if (!is_foreground_for(hwnd))
        return false;
    return send_mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, false);
}

inline bool mouse_down()
{
    return mouse_down_for(state().hwnd);
}

inline bool mouse_up()
{
    return send_mouse(MOUSEEVENTF_LEFTUP, 0, 0, false);
}

inline bool click_for(HWND hwnd, int x, int y)
{
    if (!set_foreground_for(hwnd))
        return false;
    if (!move_cursor(x, y))
        return false;
    // Moving and clicking in one SendInput batch races the hit test: Windows
    // can answer the button-down against the cursor's old position. A short
    // settle between the move, down and up was enough to make the maximize
    // button deterministic in the scripted run.
    Sleep(60);
    if (!send_mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, false))
        return false;
    Sleep(60);
    return send_mouse(MOUSEEVENTF_LEFTUP, 0, 0, false);
}

inline bool click(int x, int y)
{
    return click_for(state().hwnd, x, y);
}

inline bool click_secondary(int x, int y)
{
    SecondaryState* secondary = current_secondary();
    return secondary && click_for(secondary->hwnd, x, y);
}

inline bool secondary_mouse_down()
{
    SecondaryState* secondary = current_secondary();
    return secondary && mouse_down_for(secondary->hwnd);
}

inline std::unique_ptr<std::string> cursor_position_json()
{
    POINT point{};
    GetCursorPos(&point);
    return to_json(QJsonArray{ int(point.x), int(point.y) });
}

// ---------------------------------------------------------------------------
// Screen capture

/// Physical-pixel capture of one screen rectangle. CAPTUREBLT includes the
/// layered DWM flyouts; the whole virtual desktop is the coordinate space.
inline QImage capture_rect(int x, int y, int width, int height)
{
    if (width <= 0 || height <= 0)
        return {};

    HDC screen = GetDC(nullptr);
    if (!screen)
        return {};
    HDC memory = CreateCompatibleDC(screen);
    HBITMAP bitmap = CreateCompatibleBitmap(screen, width, height);
    if (!memory || !bitmap) {
        if (bitmap)
            DeleteObject(bitmap);
        if (memory)
            DeleteDC(memory);
        ReleaseDC(nullptr, screen);
        return {};
    }

    const HGDIOBJ previous = SelectObject(memory, bitmap);
    const BOOL copied = BitBlt(memory, 0, 0, width, height, screen, x, y, SRCCOPY | CAPTUREBLT);

    QImage image;
    if (copied) {
        image = QImage(width, height, QImage::Format_RGB32);
        BITMAPINFO info = {};
        info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        if (!GetDIBits(memory, bitmap, 0, UINT(height), image.bits(), &info, DIB_RGB_COLORS))
            image = QImage();
    }

    SelectObject(memory, previous);
    DeleteObject(bitmap);
    DeleteDC(memory);
    ReleaseDC(nullptr, screen);
    return image;
}

inline bool save_image(const QString& path, const QImage& image)
{
    if (image.isNull())
        return false;
    if (!path.isEmpty()) {
        const int slash = path.lastIndexOf(QLatin1Char('/'));
        if (slash > 0) {
            // QML's file writer creates directories; this one has to as well
            // because captures can be the first write in a run.
            QDir().mkpath(path.left(slash));
        }
    }
    return image.save(path);
}

inline bool capture(const QString& path, int x, int y, int width, int height)
{
    return save_image(path, capture_rect(x, y, width, height));
}

inline bool capture_window_for(HWND hwnd, const QString& path)
{
    if (!hwnd)
        return false;
    RECT rect;
    GetWindowRect(hwnd, &rect);
    return capture(path, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top);
}

inline bool capture_window(const QString& path)
{
    return capture_window_for(state().hwnd, path);
}

inline bool capture_secondary_window(const QString& path)
{
    SecondaryState* secondary = current_secondary();
    return secondary && capture_window_for(secondary->hwnd, path);
}

// ---------------------------------------------------------------------------
// Backdrop-switch sampling

inline void begin_samples(int x, int y, int width, int height)
{
    State& value = state();
    value.sample_x = x;
    value.sample_y = y;
    value.sample_w = width;
    value.sample_h = height;
    value.samples.clear();
}

inline double percentile(QVector<double> values, double fraction)
{
    if (values.isEmpty())
        return 0.0;
    std::sort(values.begin(), values.end());
    const qsizetype index =
        std::clamp<qsizetype>(qsizetype(fraction * double(values.size() - 1)), 0, values.size() - 1);
    return values[index];
}

inline bool take_sample(const QString& tag, const QString& path)
{
    State& value = state();
    const QImage image =
        capture_rect(value.sample_x, value.sample_y, value.sample_w, value.sample_h);
    if (image.isNull())
        return false;

    Sample sample;
    sample.at_ms = now_ms();
    sample.tag = tag;
    sample.frames = value.frames;
    sample.path = path;

    QVector<double> luma;
    luma.reserve((image.width() / 3 + 1) * (image.height() / 3 + 1));
    double sum = 0.0;
    double sum_squares = 0.0;
    for (int y = 0; y < image.height(); y += 3) {
        const QRgb* line = reinterpret_cast<const QRgb*>(image.constScanLine(y));
        for (int x = 0; x < image.width(); x += 3) {
            const double gray = qGray(line[x]);
            luma.append(gray);
            sum += gray;
            sum_squares += gray * gray;
        }
    }
    const double count = double(luma.size());
    sample.mean = count > 0 ? sum / count : 0.0;
    sample.variance = count > 0 ? sum_squares / count - sample.mean * sample.mean : 0.0;
    sample.p05 = percentile(luma, 0.05);
    sample.p95 = percentile(luma, 0.95);
    value.samples.push_back(sample);

    if (!path.isEmpty())
        save_image(path, image);
    return true;
}

inline QJsonArray sample_report_array()
{
    QJsonArray root;
    for (const Sample& sample : state().samples) {
        QJsonObject entry;
        entry["t_ms"] = double(sample.at_ms);
        entry["tag"] = sample.tag;
        entry["mean"] = sample.mean;
        entry["variance"] = sample.variance;
        entry["p05"] = sample.p05;
        entry["p95"] = sample.p95;
        entry["frames"] = double(sample.frames);
        entry["path"] = sample.path;
        root.append(entry);
    }
    return root;
}

// ---------------------------------------------------------------------------
// Snap Layouts flyout discovery

struct WindowItem
{
    HWND hwnd = nullptr;
    QString class_name;
    QString title;
    DWORD pid = 0;
    RECT rect{};
    bool visible = false;
    bool cloaked = false;
    bool own = false;
};

inline BOOL CALLBACK collect_window(HWND hwnd, LPARAM parameter)
{
    auto* items = reinterpret_cast<QVector<WindowItem>*>(parameter);
    WindowItem item;
    item.hwnd = hwnd;
    wchar_t class_name[256] = {};
    GetClassNameW(hwnd, class_name, 256);
    item.class_name = QString::fromWCharArray(class_name);
    wchar_t title[512] = {};
    GetWindowTextW(hwnd, title, 512);
    item.title = QString::fromWCharArray(title);
    GetWindowThreadProcessId(hwnd, &item.pid);
    GetWindowRect(hwnd, &item.rect);
    item.visible = IsWindowVisible(hwnd) != FALSE;
    DWORD cloaked = 0;
    if (SUCCEEDED(DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &cloaked, sizeof(cloaked))))
        item.cloaked = cloaked != 0;
    item.own = (hwnd == state().hwnd);
    items->push_back(item);
    return TRUE;
}

inline std::unique_ptr<std::string> top_windows_json()
{
    QVector<WindowItem> items;
    EnumWindows(collect_window, reinterpret_cast<LPARAM>(&items));

    QJsonArray root;
    for (const WindowItem& item : items) {
        if (item.rect.right <= item.rect.left || item.rect.bottom <= item.rect.top)
            continue;
        QJsonObject entry;
        entry["hwnd"] = QString::number(reinterpret_cast<quintptr>(item.hwnd), 16);
        entry["class"] = item.class_name;
        entry["title"] = item.title;
        entry["pid"] = double(item.pid);
        entry["rect"] = rect_json(item.rect);
        entry["visible"] = item.visible;
        entry["cloaked"] = item.cloaked;
        entry["own"] = item.own;
        root.append(entry);
    }
    return to_json(root);
}

// ---------------------------------------------------------------------------
// Snap Layouts flyout: baseline diff
//
// The Windows 11 flyout is composited by DWM/Shell, not enumerated by
// EnumWindows (the run that produced this spike saw no new top-level window),
// so visibility is measured from pixels: capture the region under the maximize
// button before the hover, then again during it. The bounding box of changed
// pixels is both the evidence that the flyout opened and the anchor the script
// clicks inside.

inline void flyout_begin(int x, int y, int width, int height)
{
    State& value = state();
    value.flyout_x = x;
    value.flyout_y = y;
    value.flyout_w = width;
    value.flyout_h = height;
    value.flyout_baseline = capture_rect(x, y, width, height);
}

inline std::unique_ptr<std::string> flyout_report_json()
{
    State& value = state();
    QJsonObject root;
    const QImage now = capture_rect(value.flyout_x, value.flyout_y, value.flyout_w, value.flyout_h);
    root["ok"] = !now.isNull() && !value.flyout_baseline.isNull()
        && now.size() == value.flyout_baseline.size();
    if (!root["ok"].toBool()) {
        root["changed"] = 0;
        root["fraction"] = 0.0;
        root["bbox"] = QJsonArray{ 0, 0, 0, 0 };
        return to_json(root);
    }

    int changed = 0;
    int total = 0;
    int left = now.width();
    int top = now.height();
    int right = 0;
    int bottom = 0;
    double sum = 0.0;
    for (int y = 0; y < now.height(); y += 2) {
        const QRgb* a = reinterpret_cast<const QRgb*>(now.constScanLine(y));
        const QRgb* b = reinterpret_cast<const QRgb*>(value.flyout_baseline.constScanLine(y));
        for (int x = 0; x < now.width(); x += 2) {
            const int delta = std::max({ std::abs(qRed(a[x]) - qRed(b[x])),
                                         std::abs(qGreen(a[x]) - qGreen(b[x])),
                                         std::abs(qBlue(a[x]) - qBlue(b[x])) });
            total += 1;
            sum += delta;
            if (delta > 16) {
                changed += 1;
                left = std::min(left, x);
                top = std::min(top, y);
                right = std::max(right, x + 1);
                bottom = std::max(bottom, y + 1);
            }
        }
    }

    root["changed"] = changed;
    root["total"] = total;
    root["fraction"] = total > 0 ? double(changed) / double(total) : 0.0;
    root["mean_delta"] = total > 0 ? sum / double(total) : 0.0;
    root["bbox"] = changed > 0
        ? QJsonArray{ value.flyout_x + left, value.flyout_y + top,
                      value.flyout_x + right, value.flyout_y + bottom }
        : QJsonArray{ 0, 0, 0, 0 };
    return to_json(root);
}

// The cxx bridge hands QML JSON strings rather than Qt types, so the QJson
// builders above stay internal and these are the four symbols the bridge sees.
inline std::unique_ptr<std::string> backdrop_state_json()
{
    return to_json(backdrop_state_object());
}

inline std::unique_ptr<std::string> window_info_json()
{
    return to_json(window_info_object());
}

inline std::unique_ptr<std::string> chrome_regions_json()
{
    return to_json(chrome_regions_object());
}

inline std::unique_ptr<std::string> chrome_stats_json()
{
    return to_json(chrome_stats_object());
}

inline std::unique_ptr<std::string> sample_report_json()
{
    return to_json(sample_report_array());
}

// ---------------------------------------------------------------------------
// Secondary-window wrappers (gate 4)

inline std::unique_ptr<std::string> secondary_info_json()
{
    SecondaryState* secondary = current_secondary();
    if (!secondary)
        return to_json(QJsonObject{});
    return to_json(window_info_for(secondary->hwnd, secondary->dpr, secondary->material,
                                   secondary->dark, secondary->dark_result,
                                   secondary->backdrop_result, secondary->mica_result,
                                   secondary->blur_behind));
}

inline std::unique_ptr<std::string> secondary_backdrop_json()
{
    SecondaryState* secondary = current_secondary();
    if (!secondary)
        return to_json(QJsonObject{});
    return to_json(backdrop_state_for(secondary->hwnd, secondary->material, secondary->dark,
                                      secondary->dark_result, secondary->backdrop_result,
                                      secondary->mica_result, secondary->blur_behind));
}

inline std::unique_ptr<std::string> secondary_regions_json()
{
    SecondaryState* secondary = current_secondary();
    if (!secondary)
        return to_json(QJsonObject{});
    return to_json(chrome_regions_for(secondary->hwnd, secondary->dpr));
}

inline std::unique_ptr<std::string> secondary_stats_json()
{
    SecondaryState* secondary = current_secondary();
    if (!secondary)
        return to_json(QJsonObject{});
    return to_json(chrome_stats_for(*secondary));
}

inline int secondary_hit_test_at(int x, int y)
{
    SecondaryState* secondary = current_secondary();
    if (!secondary || !secondary->hwnd)
        return -1;
    return int(SendMessageW(secondary->hwnd, WM_NCHITTEST, 0, MAKELPARAM(x, y)));
}

inline bool close_secondary()
{
    SecondaryState* secondary = current_secondary();
    if (!secondary || !secondary->hwnd)
        return false;
    return PostMessageW(secondary->hwnd, WM_CLOSE, 0, 0) != FALSE;
}

} // namespace dae_window_probe
