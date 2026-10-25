// Native instrumentation for gate 2 of docs/decisions/engine-qt-quick.md §5.
//
// The gate is about the QML TextInput <-> Windows IME round trip, so the probe
// watches the IMM32 side of it instead of trusting TextInput.preeditText:
//
//   * every WM_IME_* message that reaches the QQuickWindow, with the
//     composition string read straight from the HIMC (GCS_COMPSTR) and the
//     committed string (GCS_RESULTSTR);
//   * the CANDIDATEFORM / COMPOSITIONFORM Qt handed to the IME, compared with
//     the caret rectangle from QInputMethod (Qt multiplies by the DPI factor
//     and passes it as client coordinates -- see qwindowsinputcontext.cpp);
//   * visible windows owned by TextInputHost.exe or with an IME-ish class,
//     plus any window that overlaps the caret, so the actual candidate window
//     can be located from the snapshot;
//   * SendInput key injection, guarded by "our window is foreground" so a
//     lost race can never type into another application;
//   * a desktop capture (BitBlt with CAPTUREBLT, which includes layered popup
//     windows) while the candidate window is up.
//
// Header-only: cxx-qt compiles the generated translation unit and calls these
// functions directly, like frame_probe.h in bench/qt-list-spike.

#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <imm.h>

#include <QAbstractNativeEventFilter>
#include <QElapsedTimer>
#include <QGuiApplication>
#include <QImage>
#include <QInputMethod>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QMetaObject>
#include <QMouseEvent>
#include <QSysInfo>
#include <QQuickItem>
#include <QQuickWindow>
#include <QString>
#include <QStringList>
#include <QWindow>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

namespace dae_ime_probe {

struct EventRecord
{
    qint64 at_ms = 0;
    int message = 0;
    qint64 wparam = 0;
    qint64 lparam = 0;
    QString composition;
    QString result;
    QString detail;
};

struct State
{
    bool attached = false;
    QElapsedTimer clock;
    HWND hwnd = nullptr;
    std::vector<EventRecord> events;
    QString last_composition;
    QString last_result;
    QString last_detail;
    // Every GCS_RESULTSTR since the last clear, concatenated. One scenario can
    // commit in more than one step; the QML text must equal the whole run.
    QString committed;

    int send_attempts = 0;
    int send_ok = 0;
    int send_aborted = 0;
    int send_failed = 0;
    int last_send_error = 0;
    int focus_cancels = 0;
    QObject* focus_filter = nullptr;
};

inline State& state()
{
    static State value;
    return value;
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

inline QString message_name(int message)
{
    switch (message) {
    case WM_IME_STARTCOMPOSITION: return QStringLiteral("WM_IME_STARTCOMPOSITION");
    case WM_IME_ENDCOMPOSITION: return QStringLiteral("WM_IME_ENDCOMPOSITION");
    case WM_IME_COMPOSITION: return QStringLiteral("WM_IME_COMPOSITION");
    case WM_IME_SETCONTEXT: return QStringLiteral("WM_IME_SETCONTEXT");
    case WM_IME_NOTIFY: return QStringLiteral("WM_IME_NOTIFY");
    case WM_IME_CONTROL: return QStringLiteral("WM_IME_CONTROL");
    case WM_IME_COMPOSITIONFULL: return QStringLiteral("WM_IME_COMPOSITIONFULL");
    case WM_IME_SELECT: return QStringLiteral("WM_IME_SELECT");
    case WM_IME_CHAR: return QStringLiteral("WM_IME_CHAR");
    case WM_IME_REQUEST: return QStringLiteral("WM_IME_REQUEST");
    case WM_IME_KEYDOWN: return QStringLiteral("WM_IME_KEYDOWN");
    case WM_IME_KEYUP: return QStringLiteral("WM_IME_KEYUP");
    case WM_INPUTLANGCHANGE: return QStringLiteral("WM_INPUTLANGCHANGE");
    case WM_INPUTLANGCHANGEREQUEST: return QStringLiteral("WM_INPUTLANGCHANGEREQUEST");
    case WM_SETFOCUS: return QStringLiteral("WM_SETFOCUS");
    case WM_KILLFOCUS: return QStringLiteral("WM_KILLFOCUS");
    case WM_ACTIVATE: return QStringLiteral("WM_ACTIVATE");
    default: return QStringLiteral("0x") + QString::number(message, 16);
    }
}

inline QString ime_notification_name(WPARAM wparam)
{
    switch (wparam) {
    case IMN_CLOSESTATUSWINDOW: return QStringLiteral("IMN_CLOSESTATUSWINDOW");
    case IMN_OPENSTATUSWINDOW: return QStringLiteral("IMN_OPENSTATUSWINDOW");
    case IMN_CHANGECANDIDATE: return QStringLiteral("IMN_CHANGECANDIDATE");
    case IMN_CLOSECANDIDATE: return QStringLiteral("IMN_CLOSECANDIDATE");
    case IMN_OPENCANDIDATE: return QStringLiteral("IMN_OPENCANDIDATE");
    case IMN_SETCONVERSIONMODE: return QStringLiteral("IMN_SETCONVERSIONMODE");
    case IMN_SETSENTENCEMODE: return QStringLiteral("IMN_SETSENTENCEMODE");
    case IMN_SETOPENSTATUS: return QStringLiteral("IMN_SETOPENSTATUS");
    case IMN_SETCANDIDATEPOS: return QStringLiteral("IMN_SETCANDIDATEPOS");
    case IMN_SETCOMPOSITIONFONT: return QStringLiteral("IMN_SETCOMPOSITIONFONT");
    case IMN_SETCOMPOSITIONWINDOW: return QStringLiteral("IMN_SETCOMPOSITIONWINDOW");
    case IMN_SETSTATUSWINDOWPOS: return QStringLiteral("IMN_SETSTATUSWINDOWPOS");
    case IMN_GUIDELINE: return QStringLiteral("IMN_GUIDELINE");
    case IMN_PRIVATE: return QStringLiteral("IMN_PRIVATE");
    default: return QStringLiteral("IMN_0x") + QString::number(quint64(wparam), 16);
    }
}

inline QString composition_string(HIMC context, DWORD kind)
{
    if (!context)
        return {};
    const LONG bytes = ImmGetCompositionStringW(context, kind, nullptr, 0);
    if (bytes <= 0)
        return {};
    std::wstring buffer(std::size_t(bytes / 2) + 1, L'\0');
    const LONG copied = ImmGetCompositionStringW(context, kind, buffer.data(), bytes);
    if (copied <= 0)
        return {};
    return QString::fromWCharArray(buffer.data(), copied / 2);
}

inline void cancel_composition()
{
    const HWND handle = window_handle();
    if (!handle)
        return;
    HIMC context = ImmGetContext(handle);
    if (!context)
        return;
    ImmNotifyIME(context, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
    ImmReleaseContext(handle, context);
    state().focus_cancels += 1;
}

// Qt 6.12's QWindowsInputContext::reset() commits the *raw* composition
// string to the old focus object when the focus object changes. Clicking
// from a composing TextInput into another field would therefore leave
// "shi'jie" in the first field instead of cancelling the composition. The
// platform input context cannot be replaced from QML, but cancelling at the
// IMM level before Qt handles the press removes the composition first, so
// reset() has nothing to commit.
class FocusCancelFilter final : public QObject
{
public:
    explicit FocusCancelFilter(QQuickWindow* window)
        : QObject(window)
        , m_window(window)
    {
    }

    bool eventFilter(QObject*, QEvent* event) override
    {
        if (!m_window)
            return false;

        if (event->type() == QEvent::MouseButtonPress) {
            auto* mouse = static_cast<QMouseEvent*>(event);
            QQuickItem* target = m_window->contentItem()->childAt(
                mouse->position().x(), mouse->position().y());
            QQuickItem* focused = qobject_cast<QQuickItem*>(QGuiApplication::focusObject());
            for (QQuickItem* item = target; item; item = item->parentItem()) {
                if (item == focused)
                    return false; // press inside the composing item
            }
            cancel_composition();
        } else if (event->type() == QEvent::KeyPress) {
            auto* key = static_cast<QKeyEvent*>(event);
            if (key->key() == Qt::Key_Tab || key->key() == Qt::Key_Backtab)
                cancel_composition();
        }
        return false;
    }

private:
    QQuickWindow* m_window = nullptr;
};

class NativeFilter final : public QAbstractNativeEventFilter
{
public:
    bool nativeEventFilter(const QByteArray& type, void* message, qintptr*) override
    {
        if (type != "windows_generic_MSG" && type != "windows_dispatcher_MSG")
            return false;
        auto* msg = static_cast<MSG*>(message);
        if (!msg)
            return false;

        const int id = int(msg->message);
        const bool ime = (id >= WM_IME_STARTCOMPOSITION && id <= WM_IME_KEYLAST)
            || (id >= WM_IME_SETCONTEXT && id <= WM_IME_KEYUP);
        const bool focus = id == WM_INPUTLANGCHANGE || id == WM_INPUTLANGCHANGEREQUEST
            || id == WM_SETFOCUS || id == WM_KILLFOCUS || id == WM_ACTIVATE;
        if (!ime && !focus)
            return false;

        EventRecord record;
        record.at_ms = now_ms();
        record.message = id;
        record.wparam = qint64(msg->wParam);
        record.lparam = qint64(msg->lParam);
        record.detail = (id == WM_IME_NOTIFY) ? ime_notification_name(msg->wParam) : QString();

        if (id == WM_IME_COMPOSITION) {
            if (HIMC context = ImmGetContext(msg->hwnd)) {
                if (msg->lParam & GCS_COMPSTR) {
                    record.composition = composition_string(context, GCS_COMPSTR);
                    state().last_composition = record.composition;
                }
                if (msg->lParam & GCS_RESULTSTR) {
                    record.result = composition_string(context, GCS_RESULTSTR);
                    state().last_result = record.result;
                    state().committed += record.result;
                }
                if (msg->lParam & GCS_COMPREADSTR)
                    record.detail = QStringLiteral("GCS_COMPREADSTR");
                ImmReleaseContext(msg->hwnd, context);
            }
        }

        State& value = state();
        value.events.push_back(record);
        if (value.events.size() > 600)
            value.events.erase(value.events.begin(), value.events.begin() + 120);
        return false;
    }
};

inline bool attach()
{
    State& value = state();
    if (!value.clock.isValid())
        value.clock.start();
    if (!value.attached) {
        auto* filter = new NativeFilter();
        QGuiApplication::instance()->installNativeEventFilter(filter);
        value.attached = true;
    }
    value.hwnd = window_handle();
    QQuickWindow* window = quick_window();
    if (window && !value.focus_filter) {
        value.focus_filter = new FocusCancelFilter(window);
        window->installEventFilter(value.focus_filter);
    }
    return value.hwnd != nullptr;
}

inline bool is_foreground()
{
    const HWND handle = window_handle();
    return handle && GetForegroundWindow() == handle;
}

inline HWND focused_window()
{
    const HWND handle = window_handle();
    if (!handle)
        return nullptr;
    GUITHREADINFO info = {};
    info.cbSize = sizeof(info);
    const DWORD thread = GetWindowThreadProcessId(handle, nullptr);
    if (!thread || !GetGUIThreadInfo(thread, &info))
        return nullptr;
    return info.hwndFocus;
}

inline bool focus_window()
{
    QQuickWindow* window = quick_window();
    if (!window)
        return false;
    HWND handle = reinterpret_cast<HWND>(window->winId());
    state().hwnd = handle;

    HWND foreground = GetForegroundWindow();
    const DWORD foreground_thread = foreground ? GetWindowThreadProcessId(foreground, nullptr) : 0;
    const DWORD this_thread = GetCurrentThreadId();
    bool attached_input = false;
    if (foreground_thread && foreground_thread != this_thread)
        attached_input = AttachThreadInput(foreground_thread, this_thread, TRUE) != FALSE;

    if (foreground != handle) {
        ShowWindow(handle, SW_SHOW);
        BringWindowToTop(handle);
        SetForegroundWindow(handle);
        SetActiveWindow(handle);
    }
    SetFocus(handle);

    if (attached_input)
        AttachThreadInput(foreground_thread, this_thread, FALSE);

    return GetForegroundWindow() == handle && focused_window() == handle;
}

inline QJsonArray point_array(double x, double y)
{
    QJsonArray array;
    array.append(x);
    array.append(y);
    return array;
}

inline QJsonArray rect_array(const QRectF& rect)
{
    QJsonArray array;
    array.append(rect.x());
    array.append(rect.y());
    array.append(rect.width());
    array.append(rect.height());
    return array;
}

inline QJsonArray native_rect_array(const RECT& rect)
{
    QJsonArray array;
    array.append(double(rect.left));
    array.append(double(rect.top));
    array.append(double(rect.right - rect.left));
    array.append(double(rect.bottom - rect.top));
    return array;
}

inline QString style_name(DWORD style)
{
    QStringList parts;
    if (style & CFS_CANDIDATEPOS)
        parts << QStringLiteral("CFS_CANDIDATEPOS");
    if (style & CFS_EXCLUDE)
        parts << QStringLiteral("CFS_EXCLUDE");
    if (style & CFS_FORCE_POSITION)
        parts << QStringLiteral("CFS_FORCE_POSITION");
    return parts.isEmpty() ? QStringLiteral("0x") + QString::number(style, 16) : parts.join('|');
}

inline QString process_name(DWORD pid)
{
    if (!pid)
        return {};
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!process)
        return {};
    wchar_t path[MAX_PATH] = {};
    DWORD size = MAX_PATH;
    QString name;
    if (QueryFullProcessImageNameW(process, 0, path, &size)) {
        const QString full = QString::fromWCharArray(path, int(size));
        const int slash = full.lastIndexOf('\\');
        name = slash >= 0 ? full.mid(slash + 1) : full;
    }
    CloseHandle(process);
    return name;
}

inline QJsonObject window_json(HWND handle, HWND own)
{
    QJsonObject object;
    RECT rect = {};
    GetWindowRect(handle, &rect);
    wchar_t class_name[256] = {};
    GetClassNameW(handle, class_name, 256);
    wchar_t title[256] = {};
    GetWindowTextW(handle, title, 256);
    DWORD pid = 0;
    GetWindowThreadProcessId(handle, &pid);

    object["hwnd"] = double(reinterpret_cast<quintptr>(handle));
    object["own"] = handle == own;
    object["pid"] = double(pid);
    object["exe"] = process_name(pid);
    object["class"] = QString::fromWCharArray(class_name);
    object["title"] = QString::fromWCharArray(title);
    object["visible"] = IsWindowVisible(handle) != FALSE;
    object["rect"] = native_rect_array(rect);
    return object;
}

inline bool looks_like_ime_window(const QJsonObject& window)
{
    if (window.value("own").toBool())
        return false;
    const QString exe = window.value("exe").toString();
    const QString cls = window.value("class").toString();
    return exe.compare(QStringLiteral("TextInputHost.exe"), Qt::CaseInsensitive) == 0
        || cls.contains(QStringLiteral("IME"), Qt::CaseInsensitive)
        || cls.contains(QStringLiteral("Candidate"), Qt::CaseInsensitive)
        || cls.contains(QStringLiteral("TextInput"), Qt::CaseInsensitive);
}

inline QRect json_rect(const QJsonArray& array)
{
    if (array.size() != 4)
        return {};
    return QRect(int(array.at(0).toDouble()), int(array.at(1).toDouble()),
                 int(array.at(2).toDouble()), int(array.at(3).toDouble()));
}

struct EnumContext
{
    HWND own = nullptr;
    QRect near_rect;
    QJsonArray ime_windows;
    QJsonArray near_windows;
};

inline BOOL CALLBACK enum_window(HWND handle, LPARAM parameter)
{
    auto* context = reinterpret_cast<EnumContext*>(parameter);
    if (!IsWindowVisible(handle))
        return TRUE;

    QJsonObject object = window_json(handle, context->own);
    const QRect rect = json_rect(object.value("rect").toArray());
    const bool is_near = !context->near_rect.isEmpty() && rect.isValid()
        && rect.intersects(context->near_rect);
    if (is_near)
        context->near_windows.append(object);
    if (!looks_like_ime_window(object))
        return TRUE;

    context->ime_windows.append(object);

    // The candidate list can be a child of the TextInputHost window; walk one
    // level down for every IME-ish top-level window.
    struct ChildContext
    {
        HWND own = nullptr;
        QJsonArray* out = nullptr;
    } child{context->own, &context->ime_windows};
    EnumChildWindows(
        handle,
        [](HWND child_handle, LPARAM child_parameter) -> BOOL {
            auto* child_context = reinterpret_cast<ChildContext*>(child_parameter);
            child_context->out->append(window_json(child_handle, child_context->own));
            return TRUE;
        },
        reinterpret_cast<LPARAM>(&child));
    return TRUE;
}

inline QRect caret_native_rect(QQuickWindow* window, HWND handle)
{
    if (!window || !handle)
        return {};
    const qreal factor = window->devicePixelRatio();
    const QRectF logical = QGuiApplication::inputMethod()->cursorRectangle();
    POINT origin{0, 0};
    ClientToScreen(handle, &origin);
    return QRect(QPoint(origin.x + int(std::lround(logical.x() * factor)),
                        origin.y + int(std::lround(logical.y() * factor))),
                 QSize(int(std::lround(logical.width() * factor)),
                       int(std::lround(logical.height() * factor))));
}

inline QJsonObject ime_json(HWND handle, QQuickWindow* window)
{
    QJsonObject object;
    if (!handle)
        return object;

    HIMC context = ImmGetContext(handle);
    if (!context) {
        object["context"] = false;
        return object;
    }

    object["context"] = true;
    object["open"] = ImmGetOpenStatus(context) != FALSE;
    DWORD conversion = 0;
    DWORD sentence = 0;
    if (ImmGetConversionStatus(context, &conversion, &sentence)) {
        object["conversion"] = double(conversion);
        object["sentence"] = double(sentence);
    }
    object["composition"] = composition_string(context, GCS_COMPSTR);
    const LONG cursor = ImmGetCompositionStringW(context, GCS_CURSORPOS, nullptr, 0);
    object["composition_cursor"] = double(cursor > 0 ? cursor : 0);

    const qreal factor = window ? window->devicePixelRatio() : 1.0;
    const QRectF logical = window ? QGuiApplication::inputMethod()->cursorRectangle() : QRectF();
    const double expected_x = logical.x() * factor;
    const double expected_y = (logical.y() + logical.height()) * factor;
    object["expected_candidate_pt"] = point_array(expected_x, expected_y);

    QJsonArray forms;
    bool saw_exclude = false;
    double best_dx = 1e9;
    double best_dy = 1e9;
    for (DWORD index = 0; index < 4; ++index) {
        CANDIDATEFORM form = {};
        form.dwIndex = index;
        if (!ImmGetCandidateWindow(context, index, &form))
            continue;
        QJsonObject entry;
        entry["index"] = double(index);
        entry["style"] = double(form.dwStyle);
        entry["style_name"] = style_name(form.dwStyle);
        entry["pt"] = point_array(double(form.ptCurrentPos.x), double(form.ptCurrentPos.y));
        entry["rc_area"] = native_rect_array(form.rcArea);
        entry["delta"] = point_array(double(form.ptCurrentPos.x) - expected_x,
                                     double(form.ptCurrentPos.y) - expected_y);
        if (form.dwStyle & CFS_EXCLUDE) {
            saw_exclude = true;
            best_dx = std::min(best_dx, std::abs(double(form.ptCurrentPos.x) - expected_x));
            best_dy = std::min(best_dy, std::abs(double(form.ptCurrentPos.y) - expected_y));
        }
        forms.append(entry);
    }
    object["candidate_forms"] = forms;
    object["saw_exclude"] = saw_exclude;
    object["anchor_dx"] = saw_exclude ? best_dx : -1.0;
    object["anchor_dy"] = saw_exclude ? best_dy : -1.0;

    COMPOSITIONFORM composition = {};
    if (ImmGetCompositionWindow(context, &composition)) {
        QJsonObject entry;
        entry["style"] = double(composition.dwStyle);
        entry["style_name"] = style_name(composition.dwStyle);
        entry["pt"] = point_array(double(composition.ptCurrentPos.x),
                                  double(composition.ptCurrentPos.y));
        entry["rc_area"] = native_rect_array(composition.rcArea);
        object["composition_window"] = entry;
    }

    const DWORD bytes = ImmGetCandidateListW(context, 0, nullptr, 0);
    if (bytes >= sizeof(CANDIDATELIST)) {
        auto buffer = std::make_unique<char[]>(bytes);
        auto* list = reinterpret_cast<LPCANDIDATELIST>(buffer.get());
        if (ImmGetCandidateListW(context, 0, list, bytes)) {
            QJsonObject candidates;
            candidates["count"] = double(list->dwCount);
            candidates["selection"] = double(list->dwSelection);
            candidates["page_start"] = double(list->dwPageStart);
            candidates["page_size"] = double(list->dwPageSize);
            QJsonArray strings;
            for (DWORD index = 0; index < list->dwCount && index < 64; ++index) {
                const auto* text = reinterpret_cast<const wchar_t*>(buffer.get() + list->dwOffset[index]);
                strings.append(QString::fromWCharArray(text));
            }
            candidates["strings"] = strings;
            object["candidate_list"] = candidates;
            object["candidate_list_available"] = true;
        }
    }
    if (!object.contains("candidate_list"))
        object["candidate_list_available"] = false;

    ImmReleaseContext(handle, context);
    return object;
}

inline QJsonArray event_json()
{
    QJsonArray events;
    const auto& all = state().events;
    const std::size_t first = all.size() > 240 ? all.size() - 240 : 0;
    for (std::size_t index = first; index < all.size(); ++index) {
        const EventRecord& record = all[index];
        QJsonObject entry;
        entry["t_ms"] = double(record.at_ms);
        entry["message"] = double(record.message);
        entry["name"] = message_name(record.message);
        entry["wparam"] = double(record.wparam);
        entry["lparam"] = double(record.lparam);
        if (!record.detail.isEmpty())
            entry["detail"] = record.detail;
        if (!record.composition.isEmpty())
            entry["composition"] = record.composition;
        if (!record.result.isEmpty())
            entry["result"] = record.result;
        events.append(entry);
    }
    return events;
}

inline std::unique_ptr<std::string> to_json(const QJsonObject& object, bool pretty = false)
{
    const QJsonDocument document(object);
    const QByteArray bytes = pretty ? document.toJson(QJsonDocument::Indented)
                                    : document.toJson(QJsonDocument::Compact);
    return std::make_unique<std::string>(bytes.toStdString());
}

inline std::unique_ptr<std::string> snapshot()
{
    QJsonObject root;
    root["t_ms"] = double(now_ms());
    root["qt_version"] = QString::fromLatin1(qVersion());
    root["os"] = QSysInfo::prettyProductName();

    QQuickWindow* window = quick_window();
    HWND handle = window ? reinterpret_cast<HWND>(window->winId()) : state().hwnd;
    state().hwnd = handle;

    root["hwnd"] = double(reinterpret_cast<quintptr>(handle));
    root["foreground"] = handle && GetForegroundWindow() == handle;
    root["foreground_hwnd"] = double(reinterpret_cast<quintptr>(GetForegroundWindow()));
    root["focus_hwnd"] = double(reinterpret_cast<quintptr>(GetFocus()));
    root["active_hwnd"] = double(reinterpret_cast<quintptr>(GetActiveWindow()));
    root["gui_focus_hwnd"] = double(reinterpret_cast<quintptr>(focused_window()));

    QJsonObject send;
    send["attempts"] = state().send_attempts;
    send["ok"] = state().send_ok;
    send["aborted"] = state().send_aborted;
    send["failed"] = state().send_failed;
    send["last_error"] = state().last_send_error;
    send["focus_cancels"] = state().focus_cancels;
    root["send"] = send;

    QJsonObject screen;
    screen["primary"] = QJsonArray{
        0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)};
    screen["virtual"] = QJsonArray{GetSystemMetrics(SM_XVIRTUALSCREEN),
                                   GetSystemMetrics(SM_YVIRTUALSCREEN),
                                   GetSystemMetrics(SM_CXVIRTUALSCREEN),
                                   GetSystemMetrics(SM_CYVIRTUALSCREEN)};
    root["screen"] = screen;

    if (window) {
        root["dpr"] = window->devicePixelRatio();
        root["window_active"] = window->isActive();
        root["window_position"] = point_array(window->x(), window->y());
        root["window_size"] = point_array(window->width(), window->height());
        const QRectF cursor = QGuiApplication::inputMethod()->cursorRectangle();
        root["cursor_rect"] = rect_array(cursor);
        root["cursor_rect_native"] = rect_array(QRectF(cursor.x() * window->devicePixelRatio(),
                                                       cursor.y() * window->devicePixelRatio(),
                                                       cursor.width() * window->devicePixelRatio(),
                                                       cursor.height() * window->devicePixelRatio()));
    }

    QObject* focus = QGuiApplication::focusObject();
    if (focus) {
        root["focus_class"] = QString::fromLatin1(focus->metaObject()->className());
        root["focus_name"] = focus->objectName();
    }
    if (auto* item = qobject_cast<QQuickItem*>(focus)) {
        const QRectF cursor = item->inputMethodQuery(Qt::ImCursorRectangle).toRectF();
        root["item_cursor_rect"] = rect_array(cursor);
        const QPointF global = item->mapToGlobal(cursor.topLeft());
        root["item_cursor_global"] = point_array(global.x(), global.y());
    }

    if (handle) {
        POINT origin{0, 0};
        ClientToScreen(handle, &origin);
        root["client_origin"] = point_array(origin.x, origin.y);
    }

    root["ime"] = ime_json(handle, window);

    const QRect caret = caret_native_rect(window, handle);
    EnumContext enum_context;
    enum_context.own = handle;
    enum_context.near_rect = caret.adjusted(-420, -520, 420, 320);
    EnumWindows(&enum_window, reinterpret_cast<LPARAM>(&enum_context));
    root["caret_native"] = QJsonArray{caret.x(), caret.y(), caret.width(), caret.height()};
    root["ime_windows"] = enum_context.ime_windows;
    root["near_windows"] = enum_context.near_windows;

    root["events"] = event_json();
    root["last_composition"] = state().last_composition;
    root["last_result"] = state().last_result;
    root["committed"] = state().committed;
    return to_json(root);
}

inline std::unique_ptr<std::string> ensure_native_mode()
{
    QJsonObject object;
    QQuickWindow* window = quick_window();
    HWND handle = window ? reinterpret_cast<HWND>(window->winId()) : nullptr;
    object["ok"] = false;
    if (!handle) {
        object["error"] = QStringLiteral("no-window");
        return to_json(object);
    }

    HIMC context = ImmGetContext(handle);
    if (!context) {
        object["error"] = QStringLiteral("no-ime-context");
        return to_json(object);
    }

    object["ok"] = true;
    object["window"] = double(reinterpret_cast<quintptr>(handle));
    object["open_before"] = ImmGetOpenStatus(context) != FALSE;
    if (!ImmGetOpenStatus(context))
        ImmSetOpenStatus(context, TRUE);

    DWORD conversion = 0;
    DWORD sentence = 0;
    ImmGetConversionStatus(context, &conversion, &sentence);
    object["conversion_before"] = double(conversion);
    object["sentence_before"] = double(sentence);

    bool changed = false;
    if (!(conversion & IME_CMODE_NATIVE)) {
        DWORD target = (conversion | IME_CMODE_NATIVE) & ~IME_CMODE_NOCONVERSION;
        changed = ImmSetConversionStatus(context, target, sentence) != FALSE;
    }

    ImmGetConversionStatus(context, &conversion, &sentence);
    object["open_after"] = ImmGetOpenStatus(context) != FALSE;
    object["conversion_after"] = double(conversion);
    object["sentence_after"] = double(sentence);
    object["changed"] = changed;
    object["native"] = (conversion & IME_CMODE_NATIVE) != 0;

    ImmReleaseContext(handle, context);
    return to_json(object);
}

inline bool send_key(int virtual_key)
{
    HWND handle = window_handle();
    if (!handle)
        return false;
    if (GetForegroundWindow() != handle || focused_window() != handle)
        focus_window();
    // Never inject into whatever happens to be foreground: the caller treats
    // false as "abort this scenario".
    if (GetForegroundWindow() != handle || focused_window() != handle) {
        state().send_aborted += 1;
        return false;
    }

    state().send_attempts += 1;
    INPUT inputs[2] = {};
    inputs[0].type = INPUT_KEYBOARD;
    inputs[0].ki.wVk = WORD(virtual_key);
    inputs[0].ki.wScan = WORD(MapVirtualKeyW(UINT(virtual_key), MAPVK_VK_TO_VSC));
    inputs[0].ki.dwFlags = 0;
    inputs[0].ki.time = 0;
    inputs[0].ki.dwExtraInfo = 0;
    inputs[1] = inputs[0];
    inputs[1].ki.dwFlags = KEYEVENTF_KEYUP;
    if (SendInput(2, inputs, sizeof(INPUT)) == 2) {
        state().send_ok += 1;
        return true;
    }
    state().send_failed += 1;
    state().last_send_error = int(GetLastError());
    return false;
}

inline bool click_item(const QString& name)
{
    QQuickWindow* window = quick_window();
    if (!window)
        return false;
    HWND handle = reinterpret_cast<HWND>(window->winId());
    if (GetForegroundWindow() != handle || focused_window() != handle)
        focus_window();
    if (GetForegroundWindow() != handle)
        return false;

    auto* item = window->findChild<QQuickItem*>(name);
    if (!item)
        return false;

    const QPointF center(item->width() / 2.0, item->height() / 2.0);
    const QPointF scene = item->mapToScene(center);
    POINT origin{0, 0};
    ClientToScreen(handle, &origin);
    const qreal factor = window->devicePixelRatio();
    const int x = origin.x + int(std::lround(scene.x() * factor));
    const int y = origin.y + int(std::lround(scene.y() * factor));

    SetCursorPos(x, y);
    INPUT inputs[2] = {};
    inputs[0].type = INPUT_MOUSE;
    inputs[0].mi.dwFlags = MOUSEEVENTF_LEFTDOWN;
    inputs[1].type = INPUT_MOUSE;
    inputs[1].mi.dwFlags = MOUSEEVENTF_LEFTUP;
    return SendInput(2, inputs, sizeof(INPUT)) == 2;
}

inline QImage desktop_image()
{
    const int x = GetSystemMetrics(SM_XVIRTUALSCREEN);
    const int y = GetSystemMetrics(SM_YVIRTUALSCREEN);
    const int width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    const int height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
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

inline bool grab(const QString& path)
{
    const QImage desktop = desktop_image();
    if (desktop.isNull())
        return false;
    if (!desktop.save(path))
        return false;

    QQuickWindow* window = quick_window();
    HWND handle = window ? reinterpret_cast<HWND>(window->winId()) : nullptr;
    const QRect caret = caret_native_rect(window, handle);
    if (caret.isEmpty())
        return true;

    // Enough room for a candidate list above, below, left or right of the
    // caret at 150% scaling. The full desktop grab is kept next to it.
    QRect crop = caret.adjusted(-420, -520, 520, 320);
    const int origin_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
    const int origin_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
    crop.translate(-origin_x, -origin_y);
    crop = crop.intersected(desktop.rect());
    if (crop.isEmpty())
        return true;

    QString crop_path = path;
    if (crop_path.endsWith(QStringLiteral(".png"), Qt::CaseInsensitive))
        crop_path.chop(4);
    crop_path += QStringLiteral("-crop.png");
    return desktop.copy(crop).save(crop_path);
}

inline void clear_events()
{
    State& value = state();
    value.events.clear();
    value.last_composition.clear();
    value.last_result.clear();
    value.last_detail.clear();
    value.committed.clear();
}

inline std::unique_ptr<std::string> event_log()
{
    QJsonObject root;
    root["events"] = event_json();
    root["last_composition"] = state().last_composition;
    root["last_result"] = state().last_result;
    root["committed"] = state().committed;
    return to_json(root);
}

} // namespace dae_ime_probe
