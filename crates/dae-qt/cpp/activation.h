// The QML face of a forwarded launch, and the single-instance claim that must
// run before the engine is created.
//
// A macro guard rather than `#pragma once`: cxx-qt copies every moc'd header
// into the generated include tree, and `activation.cpp` reaches this file
// relatively while the generated wrapper includes that copy. `#pragma once`
// keys on the file, so it lets the second copy through and the class is
// redefined. The macro is the same in both copies and stops it.
#ifndef DAE_QT_ACTIVATION_H
#define DAE_QT_ACTIVATION_H

#include <QObject>
#include <QStringList>

namespace dae_qt {

class ActivationServer;

/// Collects the directories named by a second launch.
///
/// QML listens to `activated()` and pulls through `takePending()`. The pending
/// list is what makes a launch that arrives while the first window is still
/// being built survive; the signal alone would be emitted before QML connected.
class Activation final : public QObject
{
    Q_OBJECT

public:
    explicit Activation(QObject* parent = nullptr);

    /// Everything collected since the last call, oldest first.
    Q_INVOKABLE QStringList takePending();

signals:
    /// Emitted for every forwarded launch, including one that carried no
    /// openable path. QML then only brings the running window forward.
    void activated();

private:
    friend bool dae_qt_activation_claim();
    friend class ActivationServer;

    void enqueue(const QStringList& args, const QString& workingDirectory);
    void addPending(const QStringList& paths);

    QStringList m_pending;
};

/// Claims the process-wide single-instance slot.
///
/// The first process installs the local server, makes this object available to
/// QML as `dae.activation/Activation`, and returns true. A later launch sends
/// its arguments to the owner and returns false, so `main` exits before
/// creating an engine or a window.
bool dae_qt_activation_claim();

} // namespace dae_qt

#endif // DAE_QT_ACTIVATION_H
