#include "activation.h"

#include "dae-qt/src/activation.cxxqt.h"

#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDataStream>
#include <QDir>
#include <QLocalServer>
#include <QLocalSocket>
#include <QLockFile>
#include <QStandardPaths>
#include <QThread>
#include <QtQml/qqml.h>

#include <limits>

#ifdef Q_OS_WIN
#  ifndef NOMINMAX
#    define NOMINMAX
#  endif
#  include <windows.h>
#endif

namespace dae_qt {

namespace {

constexpr int kMaxForwardedArgs = 32;
constexpr int kMaxPendingPaths = 64;
constexpr int kMaxPayloadBytes = 64 * 1024;
constexpr char kBufferProperty[] = "dae_activation_buffer";
constexpr char kFinishedProperty[] = "dae_activation_finished";

Activation* g_activation = nullptr;
QLockFile* g_lock = nullptr;
ActivationServer* g_server = nullptr;
bool g_claimed = false;

QString protocolMagic()
{
    return QStringLiteral("dae-qt-activation-v1");
}

/// A deterministic per-user key. `qHash` is salted per process in Qt 6, so it
/// cannot be used here; the digest of the home directory differs between users
/// and stays the same across launches.
QString instanceKey()
{
    const QByteArray digest = QCryptographicHash::hash(
        QDir::homePath().toUtf8(), QCryptographicHash::Sha256).toHex().left(12);
    return QStringLiteral("dae-qt-activation-%1").arg(QString::fromLatin1(digest));
}

QString lockPath(const QString& key)
{
    QString directory = QStandardPaths::writableLocation(QStandardPaths::TempLocation);
    if (directory.isEmpty())
        directory = QDir::tempPath();
    return QDir(directory).filePath(key + QStringLiteral(".lock"));
}

/// The command line the owner should interpret: this executable is argv[0] and
/// every later launch starts with the next argument.
QStringList clientArguments()
{
    return QCoreApplication::arguments().mid(1, kMaxForwardedArgs);
}

QByteArray serialize(const QStringList& args, const QString& workingDirectory)
{
    QByteArray payload;
    QDataStream stream(&payload, QIODevice::WriteOnly);
    stream.setVersion(QDataStream::Qt_6_0);
    stream << protocolMagic() << workingDirectory << args;
    return payload;
}

bool forwardToOwner(const QString& key, const QStringList& args,
                    const QString& workingDirectory, qint64 ownerPid)
{
#ifdef Q_OS_WIN
    // The owner did not receive the user's click, so Windows would otherwise
    // only flash its taskbar button. A secondary launch usually still has the
    // foreground permission it was started with; grant it to the owner before
    // it raises its window.
    if (ownerPid > 0 && static_cast<quint64>(ownerPid) <= std::numeric_limits<DWORD>::max())
        AllowSetForegroundWindow(static_cast<DWORD>(ownerPid));
#else
    Q_UNUSED(ownerPid);
#endif

    const QByteArray payload = serialize(args, workingDirectory);
    if (payload.size() > kMaxPayloadBytes) {
        qWarning("dae: activation command line is too large to forward");
        return false;
    }

    QLocalSocket socket;
    // The owner may still be creating its server when the second process
    // starts, so retry for roughly one second before giving up.
    for (int attempt = 0; attempt < 40; ++attempt) {
        socket.connectToServer(key, QIODevice::WriteOnly);
        if (socket.waitForConnected(30))
            break;
        socket.abort();
        QThread::msleep(20);
    }
    if (socket.state() != QLocalSocket::ConnectedState) {
        qWarning("dae: could not reach the running instance");
        return false;
    }

    if (socket.write(payload) != payload.size()) {
        qWarning("dae: could not write the activation payload");
        socket.abort();
        return false;
    }
    socket.flush();
    const bool written = socket.waitForBytesWritten(1000);
    socket.disconnectFromServer();
    if (socket.state() != QLocalSocket::UnconnectedState)
        socket.waitForDisconnected(300);

    if (!written)
        qWarning("dae: activation payload was not flushed");
    return written;
}

} // namespace

class ActivationServer final : public QObject
{
public:
    ActivationServer(Activation* sink, QObject* parent)
        : QObject(parent)
        , m_sink(sink)
        , m_server(new QLocalServer(this))
    {
        m_server->setSocketOptions(QLocalServer::UserAccessOption);
        QObject::connect(m_server, &QLocalServer::newConnection, this, [this]() {
            while (QLocalSocket* socket = m_server->nextPendingConnection())
                read(socket);
        });
    }

    bool listen(const QString& key)
    {
        // The lock says the previous owner is gone, so any socket still on disk
        // is stale and must not keep this process from listening.
        QLocalServer::removeServer(key);
        return m_server->listen(key);
    }

    void close() { m_server->close(); }

private:
    void read(QLocalSocket* socket)
    {
        socket->setProperty(kBufferProperty, QByteArray());
        socket->setProperty(kFinishedProperty, false);

        QObject::connect(socket, &QLocalSocket::readyRead, socket, [socket]() {
            QByteArray buffer = socket->property(kBufferProperty).toByteArray();
            buffer.append(socket->readAll());
            if (buffer.size() > kMaxPayloadBytes) {
                socket->setProperty(kBufferProperty, QByteArray());
                qWarning("dae: dropped an oversized activation payload");
                socket->abort();
                return;
            }
            socket->setProperty(kBufferProperty, buffer);
        });

        const auto finish = [this, socket]() {
            if (socket->property(kFinishedProperty).toBool())
                return;
            socket->setProperty(kFinishedProperty, true);

            QByteArray buffer = socket->property(kBufferProperty).toByteArray();
            buffer.append(socket->readAll());
            socket->deleteLater();

            if (buffer.isEmpty())
                return;

            QDataStream stream(buffer);
            stream.setVersion(QDataStream::Qt_6_0);
            QString magic;
            QString workingDirectory;
            QStringList args;
            stream >> magic >> workingDirectory >> args;
            if (stream.status() != QDataStream::Ok || magic != protocolMagic())
                return;

            m_sink->enqueue(args.mid(0, kMaxForwardedArgs), workingDirectory);
        };

        QObject::connect(socket, &QLocalSocket::disconnected, this, finish);
        QObject::connect(
            socket, &QLocalSocket::errorOccurred, this,
            [socket, finish](QLocalSocket::LocalSocketError error) {
                if (error == QLocalSocket::PeerClosedError) {
                    finish();
                } else {
                    socket->deleteLater();
                }
            });
    }

    Activation* m_sink;
    QLocalServer* m_server;
};

Activation::Activation(QObject* parent)
    : QObject(parent)
{
}

QStringList Activation::takePending()
{
    QStringList pending;
    pending.swap(m_pending);
    return pending;
}

void Activation::enqueue(const QStringList& args, const QString& workingDirectory)
{
    const QStringList paths = dae_qt_activation_resolve(args, workingDirectory);
    addPending(paths);
    emit activated();
}

void Activation::addPending(const QStringList& paths)
{
    for (const QString& path : paths) {
        if (m_pending.size() >= kMaxPendingPaths)
            break;
        if (!m_pending.contains(path))
            m_pending.append(path);
    }
}

bool dae_qt_activation_claim()
{
    if (g_claimed)
        return true;
    g_claimed = true;

    const QString key = instanceKey();
    auto* lock = new QLockFile(lockPath(key));
    if (!lock->tryLock(0)) {
        qint64 ownerPid = 0;
        QString ownerHost;
        QString ownerApp;
        const bool ownerKnown = lock->getLockInfo(&ownerPid, &ownerHost, &ownerApp);
        forwardToOwner(key, clientArguments(), QDir::currentPath(),
                       ownerKnown ? ownerPid : 0);
        delete lock;
        return false;
    }

    g_lock = lock;
    g_activation = new Activation;

    // Register before the engine exists so QML's `import dae.activation 1.0`
    // resolves the instance. The object outlives the engine.
    qmlRegisterSingletonInstance("dae.activation", 1, 0, "Activation", g_activation);

    auto* server = new ActivationServer(g_activation, g_activation);
    g_server = server;
    if (!server->listen(key)) {
        qWarning("dae: single-instance socket is unavailable; "
                 "a second launch cannot reach this window");
    }

    // The first launch's arguments go through the same pipeline as forwarded
    // ones. Config has already chosen the startup tab; QML reuses that tab and
    // adds one for every further target.
    const QStringList initial = clientArguments();
    if (!initial.isEmpty())
        g_activation->enqueue(initial, QDir::currentPath());

    QObject::connect(qApp, &QCoreApplication::aboutToQuit, []() {
        if (g_server)
            g_server->close();
        if (g_lock)
            g_lock->unlock();
    });
    return true;
}

} // namespace dae_qt
