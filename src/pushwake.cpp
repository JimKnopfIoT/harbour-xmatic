#include "pushwake.h"

#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusInterface>
#include <QDir>
#include <QStandardPaths>
#include <QTimer>
#include <QVariantMap>

#include <sys/prctl.h>
#include <sys/stat.h>

#include "appsettings.h"
#include "instancelock.h"
#include "matrixbridge.h"
#include "secretskeeper.h"

namespace {

/// The name the connector owns. The distributor calls back on exactly the
/// string handed to it at registration, and this is that string.
const char *ConnectorName = "org.unifiedpush.Connector.xmatic";

/// How long to wait for the push after claiming the name: a ceiling for a busy
/// device, not an expectation.
const int MessageWaitMs = 20000;

/// And for the round trip that fetches the message behind it. Longer, because
/// it is a network request on a phone that may just have woken up.
const int FetchWaitMs = 30000;

/// Raises the banner without Qt Quick - this process has no QML engine. The
/// category is what makes it audible; without one the banner is silent.
void publishBanner(const QString &summary, const QString &body, bool noisy)
{
    QDBusInterface notifications(QStringLiteral("org.freedesktop.Notifications"),
                                 QStringLiteral("/org/freedesktop/Notifications"),
                                 QStringLiteral("org.freedesktop.Notifications"),
                                 QDBusConnection::sessionBus());
    if (!notifications.isValid()) {
        qWarning("xmatic: no notification service on the session bus");
        return;
    }

    QVariantMap hints;
    // The category carries tone, vibration and LED. Left off where the push
    // rules call this one quiet - the preview hints still show the banner.
    if (noisy) {
        hints.insert(QStringLiteral("category"), QStringLiteral("x-nemo.messaging.im"));
    }
    // Summary and body alone only fill the event feed. The banner that slides
    // in over whatever is on screen is this pair.
    hints.insert(QStringLiteral("x-nemo-preview-summary"), summary);
    hints.insert(QStringLiteral("x-nemo-preview-body"), body);
    hints.insert(QStringLiteral("x-nemo-owner"), QStringLiteral("xmatic"));

    notifications.call(QStringLiteral("Notify"),
                       QStringLiteral("xmatic"),
                       uint(0),
                       QStringLiteral("/usr/share/icons/hicolor/86x86/apps/harbour-xmatic.png"),
                       summary,
                       body,
                       QStringList(),
                       hints,
                       -1);
}

QString ensureDirectory(QStandardPaths::StandardLocation location)
{
    const QString path = QStandardPaths::writableLocation(location);
    if (path.isEmpty() || !QDir().mkpath(path)) {
        return QString();
    }
    return path;
}

} // namespace

int runPushWake(int argc, char *argv[])
{
    // The same reasoning as the app's own: this process opens the crypto
    // store and holds an access token.
    prctl(PR_SET_DUMPABLE, 0);
    // Same as the app: this process opens the same stores.
    umask(S_IRWXG | S_IRWXO);

    QCoreApplication app(argc, argv);

    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) {
        qWarning("xmatic: push wake-up has no session bus");
        return 0;
    }

    // The running app owns this name and got the push. Checked rather than
    // attempted: both bus libraries queue for a contended name instead of failing.
    if (bus.interface()->isServiceRegistered(QString::fromLatin1(ConnectorName))) {
        qInfo("xmatic: the app is running and holds the connector; nothing to wake");
        return 0;
    }

    const QString dataDirectory = ensureDirectory(QStandardPaths::AppDataLocation);
    if (dataDirectory.isEmpty()) {
        qWarning("xmatic: push wake-up has no writable data directory");
        return 0;
    }

    // One process per store: if the app runs without owning the connector name,
    // this process must not open the same SQLite files behind it.
    if (!acquireInstanceLock(dataDirectory)) {
        qInfo("xmatic: another instance owns this store; not waking for a push");
        return 0;
    }

    const QString cacheDirectory = ensureDirectory(QStandardPaths::CacheLocation);
    if (cacheDirectory.isEmpty()) {
        qWarning("xmatic: push wake-up has no writable cache directory");
        return 0;
    }

    // The Secrets collection is device-lock-bound and a background activation
    // cannot answer its dialog. A banner without content is still true.
    StoreKeyResult storeKey = obtainStoreKey(dataDirectory);
    const bool locked = storeKey.state != StoreKeyState::Available;
    if (locked) {
        qInfo("xmatic: push wake-up without a store key (state %d)",
              static_cast<int>(storeKey.state));
    }

    AppSettings settings;
    MatrixBridge bridge(dataDirectory, cacheDirectory, storeKey, &settings);
    if (!storeKey.key.isEmpty()) {
        storeKey.key.fill(QChar('0'));
    }

    // Owned so the distributor's callback lands here. The core's connector
    // claims it as soon as the first push command reaches it.
    bridge.refreshPushStatus();

    bool published = false;
    const QString genericSummary = QCoreApplication::translate("PushWake", "New message");

    // The push named a room and an event; ask for the message. Where it cannot be
    // had, the banner still says something arrived.
    QObject::connect(&bridge, &MatrixBridge::pushNotificationReady, &app,
                     [&](const QVariantMap &notification) {
        if (published) {
            return;
        }
        published = true;
        const QString room = notification.value(QStringLiteral("roomName")).toString();
        const QString body = notification.value(QStringLiteral("body")).toString();
        publishBanner(room.isEmpty() ? genericSummary : room,
                      body.isEmpty() ? genericSummary : body,
                      notification.value(QStringLiteral("noisy")).toBool());
        app.quit();
    });

    QObject::connect(&bridge, &MatrixBridge::pushNotificationFailed, &app,
                     [&](const QString &reason) {
        if (published) {
            return;
        }
        // "Filtered out" is the push rules saying do not show it. Anything else is a
        // failure to look, and silence would hide an arrival.
        if (reason == QLatin1String("filtered out")
                || reason == QLatin1String("redacted")) {
            qInfo("xmatic: push not shown (%s)", qPrintable(reason));
            published = true;
            app.quit();
            return;
        }
        published = true;
        qWarning("xmatic: push could not be fetched (%s)", qPrintable(reason));
        publishBanner(genericSummary, genericSummary, false);
        app.quit();
    });

    // Two ceilings, because two things can fail to happen: the distributor may
    // never call, and the fetch may never answer.
    QTimer::singleShot(MessageWaitMs, &app, [&]() {
        if (!bridge.pushMessageSeen()) {
            qInfo("xmatic: no push arrived within the wait; leaving");
            app.quit();
        }
    });
    QTimer::singleShot(MessageWaitMs + FetchWaitMs, &app, [&]() {
        if (!published) {
            qWarning("xmatic: the push could not be turned into a notification in time");
            publishBanner(genericSummary, genericSummary, false);
            app.quit();
        }
    });

    return app.exec();
}
