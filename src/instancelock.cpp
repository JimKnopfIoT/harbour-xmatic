// Single-instance enforcement.
//
// The core runs its SQLite stores with CrossProcessLockConfig::SingleProcess
// (core/src/session.rs), safe only with one process per store: two race for
// single-use OAuth refresh tokens and write the crypto store under each other.
//
// The icon tap is covered without this file — lipstick launches even a generic
// application through `invoker --single-instance` (verified on the device). What
// is not covered is D-Bus activation: the share service and a notification start
// the app from org.xmatic.xmatic.service, past the invoker, and the name that
// would deduplicate them is only owned once QML has loaded.
//
// flock() rather than a PID file: the kernel releases it when the process dies,
// whichever way it dies, so there is no stale lock to reason about after a
// SIGKILL and no PID to compare against a recycled one.

#include "instancelock.h"

#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusReply>
#include <QFile>

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>

namespace {

// Deliberately never closed: the lock lives exactly as long as the process.
int lockDescriptor = -1;

} // namespace

bool acquireInstanceLock(const QString &dataDirectory)
{
    if (dataDirectory.isEmpty()) {
        // Without a data directory there is no store to protect either; the
        // caller already warned about it.
        return true;
    }

    const QByteArray path = QFile::encodeName(dataDirectory + QStringLiteral("/instance.lock"));
    const int fd = ::open(path.constData(), O_RDWR | O_CREAT | O_CLOEXEC, 0600);
    if (fd < 0) {
        // A sandbox or a full filesystem must not keep the app from starting;
        // the risk this guards against needs a second process to materialise,
        // the failure here is certain.
        qWarning("xmatic: instance lock cannot be opened (%s); starting unguarded",
                 strerror(errno));
        return true;
    }

    if (::flock(fd, LOCK_EX | LOCK_NB) != 0) {
        if (errno == EWOULDBLOCK) {
            ::close(fd);
            return false;
        }
        // Some filesystems have no flock at all. Same reasoning as above.
        qWarning("xmatic: instance lock cannot be taken (%s); starting unguarded",
                 strerror(errno));
        ::close(fd);
        return true;
    }

    lockDescriptor = fd;
    return true;
}

void raiseRunningInstance()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) {
        return;
    }

    // Whether the name is owned is asked first, and the answer decides whether
    // anything is sent at all. A method call to an unowned name does not fail —
    // the bus starts the service from org.xmatic.xmatic.service, which is this
    // very binary. That third process would find the lock held too and call
    // again: a start-up loop built out of the handover meant to prevent one.
    //
    // The name is unowned exactly while the running instance is still loading
    // QML, and then nothing needs sending anyway: it is on its way to the
    // screen, and a share that arrived by activation is queued by the bus and
    // delivered as soon as it takes the name.
    QDBusConnectionInterface *bus_interface = bus.interface();
    if (!bus_interface
        || !bus_interface->isServiceRegistered(QStringLiteral("org.xmatic.xmatic")).value()) {
        return;
    }

    // The name, path and method are the ones the notification's remote action
    // and the share service already use.
    QDBusMessage activate =
        QDBusMessage::createMethodCall(QStringLiteral("org.xmatic.xmatic"),
                                       QStringLiteral("/org/xmatic/xmatic"),
                                       QStringLiteral("org.xmatic.xmatic"),
                                       QStringLiteral("activate"));
    // Blocking, with a short timeout: this process exits right afterwards, and
    // a fire-and-forget message can still be sitting in the outgoing queue when
    // the connection goes down with it. Two seconds is long enough for a call
    // that only raises a window and short enough not to look like a hang.
    bus.call(activate, QDBus::Block, 2000);
}
