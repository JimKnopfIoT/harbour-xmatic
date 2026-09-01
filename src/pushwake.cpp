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

#include "appsettings.h"
#include "instancelock.h"
#include "matrixbridge.h"
#include "secretskeeper.h"

namespace {

/// The name the connector owns. The distributor calls back on exactly the
/// string handed to it at registration, and this is that string.
const char *ConnectorName = "org.unifiedpush.Connector.xmatic";

/// How long to wait for the push after claiming the name. The distributor
/// calls immediately after activating us; this is the ceiling for a device
/// that is busy, not an expectation.
const int MessageWaitMs = 20000;

/// And for the round trip that fetches the message behind it. Longer, because
/// it is a network request on a phone that may just have woken up.
const int FetchWaitMs = 30000;

/// Raises the banner without Qt Quick.
///
/// Not the `Notification` QML type: this process has no QML engine, and adding
/// one to show a line of text would mean starting the whole UI stack for a
/// notification. `org.freedesktop.Notifications` is what that type talks to
/// anyway, and the hints below are what lipstick reads.
///
/// The category is what makes it audible at all. On Sailfish the tone, the
/// vibration and the LED hang off the category and not off the notification,
/// so without one the banner is silent — the same lesson the app's own
/// notifications carry.
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
    hints.insert(QStringLiteral("category"),
                 noisy ? QStringLiteral("x-nemo.messaging.im")
                       : QStringLiteral("x-nemo.messaging.im"));
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

    QCoreApplication app(argc, argv);

    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) {
        qWarning("xmatic: push wake-up has no session bus");
        return 0;
    }

    // The running app owns this name and has the registration; the distributor
    // will have delivered to it and there is nothing to do here. Checked
    // rather than attempted: zbus and QtDBus both *queue* for a contended
    // name rather than failing, so a second connector would register and then
    // wait for a callback that goes to the first one.
    if (bus.interface()->isServiceRegistered(QString::fromLatin1(ConnectorName))) {
        qInfo("xmatic: the app is running and holds the connector; nothing to wake");
        return 0;
    }

    const QString dataDirectory = ensureDirectory(QStandardPaths::AppDataLocation);
    if (dataDirectory.isEmpty()) {
        qWarning("xmatic: push wake-up has no writable data directory");
        return 0;
    }

    // One process per store, here as everywhere. If the app is running without
    // owning the connector name — it may not have push turned on — this
    // process must not open the same SQLite files behind it.
    if (!acquireInstanceLock(dataDirectory)) {
        qInfo("xmatic: another instance owns this store; not waking for a push");
        return 0;
    }

    const QString cacheDirectory = ensureDirectory(QStandardPaths::CacheLocation);

    // The one that can genuinely fail here. The Secrets collection is
    // device-lock-bound and secretsd unlocks it only through its own system
    // dialog — which a background activation cannot answer. So after a reboot,
    // before the app has been opened by hand once, there is no key and nothing
    // can be decrypted.
    //
    // That is not a reason to say nothing. The push named a room, so a banner
    // without content is still true and still useful; a silent phone would
    // leave the user believing nothing arrived.
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

    // The push arrived and named a room and an event. Ask for the message
    // behind it; where that cannot be had — no key, a filtered event, a server
    // that no longer has it — the banner still says something arrived.
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
        // "Filtered out" is the push rules saying this one is not to be shown.
        // Anything else is a failure to look, and silence would hide an
        // arrival the user was told they would hear about.
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

    // Two ceilings, because two different things can fail to happen: the
    // distributor may never call, and the fetch may never answer. Without them
    // this process would sit in memory for ever on a phone whose owner is
    // asleep.
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
