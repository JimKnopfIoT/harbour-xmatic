#include "pushwake.h"

#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusInterface>
#include <QDBusReply>
#include <QDataStream>
#include <QUuid>
#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QStandardPaths>
#include <QTimer>
#include <QVariantMap>

#include <sys/prctl.h>
#include <sys/stat.h>

#include "appsettings.h"
#include "instancelock.h"
#include "matrixbridge.h"
#include "secretskeeper.h"
#include "xmatic_core.h"

namespace {

/// The name the connector owns. The distributor calls back on exactly the
/// string handed to it at registration, and this is that string.
const char *ConnectorName = "org.unifiedpush.Connector.xmatic";

/// And for the round trip that fetches the message behind it. Longer, because
/// it is a network request on a phone that may just have woken up.
const int FetchWaitMs = 30000;

/// Banner ids by room id, so a room's banner is replaced, not stacked.
QString bannerRecordPath(const QString &dataDirectory)
{
    return dataDirectory + QStringLiteral("/push-banners.json");
}

QJsonObject readBannerRecord(const QString &dataDirectory)
{
    QFile file(bannerRecordPath(dataDirectory));
    if (!file.open(QIODevice::ReadOnly)) {
        return QJsonObject();
    }
    return QJsonDocument::fromJson(file.readAll()).object();
}

void writeBannerRecord(const QString &dataDirectory, const QJsonObject &record)
{
    if (record.isEmpty()) {
        QFile::remove(bannerRecordPath(dataDirectory));
        return;
    }
    QSaveFile file(bannerRecordPath(dataDirectory));
    if (file.open(QIODevice::WriteOnly)) {
        file.write(QJsonDocument(record).toJson(QJsonDocument::Compact));
        file.commit();
    }
}

/// Raises the banner without Qt Quick - this process has no QML engine. The
/// category is what makes it audible; without one the banner is silent.
void publishBanner(const QString &dataDirectory, const QString &roomId,
                   const QString &summary, const QString &body, bool noisy)
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

    QJsonObject record = readBannerRecord(dataDirectory);
    const QString slot = roomId.isEmpty() ? QStringLiteral("*") : roomId;
    QJsonObject entry = record.value(slot).toObject();
    const uint replaces = uint(entry.value(QStringLiteral("id")).toDouble());

    // The action carries an opaque key, not the room id; the app maps it back.
    QString action = QStringLiteral("org.xmatic.xmatic /org/xmatic/xmatic org.xmatic.xmatic activate");
    if (!roomId.isEmpty()) {
        QString key = entry.value(QStringLiteral("key")).toString();
        if (key.isEmpty()) {
            key = QString::fromLatin1(QUuid::createUuid().toRfc4122().toHex());
            entry.insert(QStringLiteral("key"), key);
        }
        QByteArray argument;
        QDataStream stream(&argument, QIODevice::WriteOnly);
        stream << QVariant(key);
        action = QStringLiteral("org.xmatic.xmatic /org/xmatic/xmatic org.xmatic.xmatic openPushBanner ")
                + QString::fromLatin1(argument.toBase64());
    }
    hints.insert(QStringLiteral("x-nemo-remote-action-default"), action);
    hints.insert(QStringLiteral("x-nemo-remote-action-app"), action);

    const QDBusReply<uint> reply = notifications.call(QStringLiteral("Notify"),
                       QStringLiteral("xmatic"),
                       replaces,
                       QStringLiteral("/usr/share/icons/hicolor/86x86/apps/harbour-xmatic.png"),
                       summary,
                       body,
                       QStringList{QStringLiteral("default"), QString(),
                                   QStringLiteral("app"), QString()},
                       hints,
                       -1);
    if (reply.isValid()) {
        entry.insert(QStringLiteral("id"), double(reply.value()));
        record.insert(slot, entry);
        writeBannerRecord(dataDirectory, record);
    }
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
    // The same reasoning as the app's own: this process holds the push keys and
    // shares the data directory.
    prctl(PR_SET_DUMPABLE, 0);
    // Same as the app: this process opens the same stores.
    umask(S_IRWXG | S_IRWXO);

    QCoreApplication app(argc, argv);
    // The desktop file's names, which the app gets from SailfishApp: without
    // them AppDataLocation is another directory and no store is found.
    QCoreApplication::setOrganizationName(QStringLiteral("org.xmatic"));
    QCoreApplication::setApplicationName(QStringLiteral("xmatic"));

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

    // One process per store: the wake-up opens none, but the app must not start
    // behind it unasked; it asks through the yield file instead.
    if (!acquireInstanceLock(dataDirectory)) {
        qInfo("xmatic: another instance owns this store; not waking for a push");
        return 0;
    }

    // Claim the name before anything slow: Foghorn allows 8 s per call.
    const QJsonObject wake = [] {
        char *raw = xm_push_wake();
        if (!raw) {
            return QJsonObject();
        }
        const QJsonObject parsed = QJsonDocument::fromJson(QByteArray(raw)).object();
        xm_string_free(raw);
        return parsed;
    }();
    const QJsonArray messages = wake.value(QStringLiteral("messages")).toArray();

    if (wake.value(QStringLiteral("unregistered")).toBool()) {
        qInfo("xmatic: the distributor unregistered this app");
        publishPushNotice(dataDirectory);
    }
    if (messages.isEmpty()) {
        qInfo("xmatic: push wake with no message to fetch (new address: %d)",
              wake.value(QStringLiteral("newEndpoint")).toBool() ? 1 : 0);
        return 0;
    }
    qInfo("xmatic: push wake with %d message(s) to fetch", int(messages.size()));

    const QString cacheDirectory = ensureDirectory(QStandardPaths::CacheLocation);
    if (cacheDirectory.isEmpty()) {
        qWarning("xmatic: push wake-up has no writable cache directory");
        publishBanner(dataDirectory, QString(),
                      QCoreApplication::translate("PushWake", "New message"),
                      QCoreApplication::translate("PushWake", "New message"), true);
        return 0;
    }

    // The Secrets collection is device-lock-bound and a background activation
    // cannot answer its dialog. A banner without content is still true.
    StoreKeyResult storeKey = obtainStoreKey(dataDirectory);
    if (storeKey.state != StoreKeyState::Available) {
        qInfo("xmatic: push wake-up without a store key (state %d)",
              static_cast<int>(storeKey.state));
    }

    AppSettings settings;
    MatrixBridge bridge(dataDirectory, cacheDirectory, storeKey, &settings);
    if (!storeKey.key.isEmpty()) {
        storeKey.key.fill(QChar('0'));
    }
    bridge.restoreSession();

    const QString genericSummary = QCoreApplication::translate("PushWake", "New message");

    const int expected = messages.size();
    int settled = 0;
    auto settle = [&]() {
        if (++settled >= expected) {
            app.quit();
        }
    };

    QObject::connect(&bridge, &MatrixBridge::pushNotificationReady, &app,
                     [&](const QVariantMap &notification) {
        const QString room = notification.value(QStringLiteral("roomName")).toString();
        const QString body = notification.value(QStringLiteral("body")).toString();
        publishBanner(dataDirectory,
                      notification.value(QStringLiteral("roomId")).toString(),
                      room.isEmpty() ? genericSummary : room,
                      body.isEmpty() ? genericSummary : body,
                      notification.value(QStringLiteral("noisy")).toBool());
        settle();
    });

    QObject::connect(&bridge, &MatrixBridge::pushNotificationFailed, &app,
                     [&](const QString &reason) {
        if (reason == QLatin1String("filtered out")
                || reason == QLatin1String("redacted")) {
            qInfo("xmatic: push not shown (%s)", qPrintable(reason));
        } else {
            qWarning("xmatic: push could not be fetched (%s)", qPrintable(reason));
            publishBanner(dataDirectory, QString(), genericSummary, genericSummary, true);
        }
        settle();
    });

    auto finishNow = [&]() {
        if (settled < expected) {
            publishBanner(dataDirectory, QString(), genericSummary, genericSummary, true);
        }
        app.quit();
    };

    // The app, started meanwhile, asks for the store: it gets it at once.
    QTimer yieldCheck;
    QObject::connect(&yieldCheck, &QTimer::timeout, &app, [&]() {
        // Only looked at: the app removes it once it holds the store.
        if (QFile::exists(dataDirectory + QStringLiteral("/") + QStringLiteral(XMATIC_WAKE_YIELD_FILE))) {
            qInfo("xmatic: the app is starting; leaving the store to it");
            finishNow();
        }
    });
    yieldCheck.start(250);

    QTimer::singleShot(FetchWaitMs, &app, [&]() {
        if (settled < expected) {
            qWarning("xmatic: %d push(es) could not be turned into a notification in time",
                     expected - settled);
        }
        finishNow();
    });

    for (const QJsonValue &message : messages) {
        const QJsonObject target = message.toObject();
        bridge.fetchPush(target.value(QStringLiteral("roomId")).toString(),
                         target.value(QStringLiteral("eventId")).toString());
    }

    return app.exec();
}

QHash<QString, QString> pushBannerRooms(const QString &dataDirectory)
{
    QHash<QString, QString> rooms;
    const QJsonObject record = readBannerRecord(dataDirectory);
    for (auto it = record.constBegin(); it != record.constEnd(); ++it) {
        const QString key = it.value().toObject().value(QStringLiteral("key")).toString();
        if (!key.isEmpty()) {
            rooms.insert(key, it.key());
        }
    }
    return rooms;
}

void closePushBanners(const QString &dataDirectory)
{
    const QJsonObject record = readBannerRecord(dataDirectory);
    if (record.isEmpty()) {
        return;
    }
    QDBusInterface notifications(QStringLiteral("org.freedesktop.Notifications"),
                                 QStringLiteral("/org/freedesktop/Notifications"),
                                 QStringLiteral("org.freedesktop.Notifications"),
                                 QDBusConnection::sessionBus());
    if (notifications.isValid()) {
        for (auto it = record.constBegin(); it != record.constEnd(); ++it) {
            notifications.call(QStringLiteral("CloseNotification"),
                               uint(it.value().toObject().value(QStringLiteral("id")).toDouble()));
        }
    }
    writeBannerRecord(dataDirectory, QJsonObject());
}

void publishPushNotice(const QString &dataDirectory)
{
    publishBanner(dataDirectory, QStringLiteral("unregistered"),
                  QStringLiteral("xmatic"),
                  QCoreApplication::translate(
                      "PushWake",
                      "The push distributor stopped delivering to xmatic. Switch push notifications on again under Account."),
                  false);
}
