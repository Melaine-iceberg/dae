// Per-frame work counters for the 120k list spike.
//
// `frameSwapped` intervals alone cannot answer the gate on a 120 Hz display:
// an empty list and a 120k-row list both present at ~8.3 ms because the
// presenter waits for the display. These hooks measure the two halves of the
// work that happens *inside* that interval:
//
//   * render thread: beforeSynchronizing -> afterSynchronizing,
//                    beforeRendering     -> afterRendering,
//                    and the total of both;
//   * GUI thread:    event-dispatcher busy time between two frameSwapped
//                    callbacks, i.e. the time the GUI thread spent processing
//                    instead of waiting. Delegate reuse, bindings and the
//                    scroll handler all live there.
//
// Everything is header-only so the cxx-qt generated translation unit can call
// it without a second build step. The render-thread hooks use DirectConnection:
// they must run on the rendering thread, not be queued to the GUI thread.

#pragma once

#include <QAbstractEventDispatcher>
#include <QElapsedTimer>
#include <QGuiApplication>
#include <QMutex>
#include <QQuickWindow>
#include <QWindow>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace dae_spike_probe {

enum Series {
    Sync = 0,
    Render = 1,
    Total = 2,
    FrameInterval = 3,
    SeriesCount = 4,
};

struct State {
    QElapsedTimer clock;
    QMutex mutex;
    std::vector<double> series[SeriesCount];

    std::int64_t before_sync = -1;
    std::int64_t before_render = -1;
    std::int64_t last_frame = -1;

    std::int64_t gui_awake = -1;
    std::int64_t gui_busy_ns = 0;

    bool recording = false;
    bool attached = false;
};

inline State& state()
{
    static State value;
    return value;
}

inline std::int64_t now_ns()
{
    return state().clock.nsecsElapsed();
}

inline void push(Series series, std::int64_t duration_ns)
{
    State& value = state();
    if (duration_ns < 0) {
        return;
    }
    QMutexLocker locker(&value.mutex);
    if (value.recording) {
        value.series[series].push_back(double(duration_ns) / 1e6);
    }
}

inline void dae_spike_attach()
{
    State& value = state();
    if (value.attached) {
        return;
    }

    QQuickWindow* window = nullptr;
    for (QWindow* candidate : QGuiApplication::topLevelWindows()) {
        if (auto* quick = qobject_cast<QQuickWindow*>(candidate)) {
            window = quick;
            break;
        }
    }
    if (!window) {
        return;
    }

    value.attached = true;
    value.clock.start();

    QObject::connect(
        window, &QQuickWindow::beforeSynchronizing, window,
        [] { state().before_sync = now_ns(); }, Qt::DirectConnection);
    QObject::connect(
        window, &QQuickWindow::afterSynchronizing, window,
        [] {
            State& s = state();
            push(Sync, now_ns() - s.before_sync);
        },
        Qt::DirectConnection);
    QObject::connect(
        window, &QQuickWindow::beforeRendering, window,
        [] { state().before_render = now_ns(); }, Qt::DirectConnection);
    QObject::connect(
        window, &QQuickWindow::afterRendering, window,
        [] {
            State& s = state();
            const std::int64_t now = now_ns();
            push(Render, now - s.before_render);
            push(Total, now - s.before_sync);
        },
        Qt::DirectConnection);
    QObject::connect(
        window, &QQuickWindow::frameSwapped, window,
        [] {
            State& s = state();
            const std::int64_t now = now_ns();
            push(FrameInterval, now - s.last_frame);
            s.last_frame = now;
        },
        Qt::DirectConnection);

    QAbstractEventDispatcher* dispatcher = QAbstractEventDispatcher::instance();
    if (dispatcher) {
        QObject::connect(
            dispatcher, &QAbstractEventDispatcher::awake, dispatcher,
            [] { state().gui_awake = now_ns(); }, Qt::DirectConnection);
        QObject::connect(
            dispatcher, &QAbstractEventDispatcher::aboutToBlock, dispatcher,
            [] {
                State& s = state();
                if (s.gui_awake >= 0) {
                    s.gui_busy_ns += now_ns() - s.gui_awake;
                    s.gui_awake = -1;
                }
            },
            Qt::DirectConnection);
    }
}

inline void dae_spike_begin()
{
    State& value = state();
    QMutexLocker locker(&value.mutex);
    for (auto& samples : value.series) {
        samples.clear();
    }
    value.before_sync = -1;
    value.before_render = -1;
    value.last_frame = -1;
    value.gui_awake = -1;
    value.gui_busy_ns = 0;
    value.recording = true;
}

inline void dae_spike_end()
{
    State& value = state();
    QMutexLocker locker(&value.mutex);
    value.recording = false;
}

inline std::int64_t dae_spike_gui_busy_ns()
{
    return state().gui_busy_ns;
}

inline int dae_spike_count(int series)
{
    if (series < 0 || series >= SeriesCount) {
        return 0;
    }
    State& value = state();
    QMutexLocker locker(&value.mutex);
    return int(value.series[series].size());
}

inline double dae_spike_percentile(int series, double percentile)
{
    if (series < 0 || series >= SeriesCount) {
        return 0.0;
    }
    State& value = state();
    QMutexLocker locker(&value.mutex);
    const auto& samples = value.series[series];
    if (samples.empty()) {
        return 0.0;
    }
    std::vector<double> sorted = samples;
    std::sort(sorted.begin(), sorted.end());
    const auto rank = std::size_t(
        std::max(1.0, std::ceil(percentile * double(sorted.size()))));
    return sorted[std::min(rank, sorted.size()) - 1];
}

inline double dae_spike_max(int series)
{
    return dae_spike_percentile(series, 1.0);
}

inline double dae_spike_mean(int series)
{
    if (series < 0 || series >= SeriesCount) {
        return 0.0;
    }
    State& value = state();
    QMutexLocker locker(&value.mutex);
    const auto& samples = value.series[series];
    if (samples.empty()) {
        return 0.0;
    }
    double total = 0.0;
    for (double sample : samples) {
        total += sample;
    }
    return total / double(samples.size());
}

} // namespace dae_spike_probe
