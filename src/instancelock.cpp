// Single instance, because the core runs its stores with `SingleProcess`. The
// invoker covers the icon tap; D-Bus activation goes past it.

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
        // A sandbox or a full filesystem must not keep the app from starting: the risk
        // needs a second process, this failure is certain.
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

    // Whether the name is owned is asked first: a call to an unowned name starts
    // this very binary again, which is a start-up loop built from the handover.
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
    // Blocking with a short timeout: this process exits right after, and a
    // fire-and-forget message can still sit in the queue when the connection goes.
    bus.call(activate, QDBus::Block, 2000);
}
