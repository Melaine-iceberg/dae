#include "platform_watch.h"

#include <QGuiApplication>
#include <QObject>
#include <QStyleHints>

// Declares dae_qt_theme_changed, the Rust half of the bridge.
#include "dae-qt/src/workspace.cxxqt.h"

namespace dae_qt {
namespace {

class ThemeWatcher final : public QObject
{
public:
    ThemeWatcher()
        : QObject(qApp)
    {
        QObject::connect(QGuiApplication::styleHints(),
                         &QStyleHints::colorSchemeChanged, this,
                         [](Qt::ColorScheme) { report(); });
        // The startup value is already read by Workspace; reporting once more
        // closes the gap between singleton construction and watcher install.
        report();
    }

private:
    static void report()
    {
        const Qt::ColorScheme scheme =
            QGuiApplication::styleHints()->colorScheme();
        if (scheme == Qt::ColorScheme::Unknown)
            return; // the platform has no preference; keep the current reading
        dae_qt_theme_changed(scheme == Qt::ColorScheme::Dark);
    }
};

} // namespace

void dae_qt_install_theme_watcher()
{
    static ThemeWatcher* watcher = nullptr;
    if (watcher == nullptr)
        watcher = new ThemeWatcher();
}

} // namespace dae_qt
