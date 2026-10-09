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
#include <QFileInfo>
#include <QMutex>
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

/// Past Leghorn's own limits: 25 s for the first push, a minute per hold.
const int WakeLimitMs = 120000;

/// Lines from the core, handed over from its worker threads.
QMutex inboxLock;
QStringList inbox;

void deliver(void *, const char *json)
{
    if (json) {
        QMutexLocker locked(&inboxLock);
        inbox.append(QString::fromUtf8(json));
    }
}

quint64 sendCommand(XmCore *core, quint64 id, const QString &command,
                    QJsonObject arguments = QJsonObject())
{
    arguments.insert(QStringLiteral("id"), double(id));
    arguments.insert(QStringLiteral("cmd"), command);
    const QByteArray payload = QJsonDocument(arguments).toJson(QJsonDocument::Compact);
    xm_core_send(core, payload.constData());
    return id;
}

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
/// category is what makes it audible; without one the banner is silent. A
/// banner with a room opens it; one without opens the app.
void publishBanner(const QString &dataDirectory, const QString &slot, const QString &roomId,
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
    const QString cacheDirectory = ensureDirectory(QStandardPaths::CacheLocation);
    if (dataDirectory.isEmpty() || cacheDirectory.isEmpty()) {
        qWarning("xmatic: push wake-up has no writable data directory");
        return 0;
    }

    // One process per store. The app asks for it through the yield file.
    if (!acquireInstanceLock(dataDirectory)) {
        qInfo("xmatic: another instance owns this store; not waking for a push");
        return 0;
    }

    // Only a key secretsd gives without a dialog. Without it, banners say "New message".
    StoreKeyResult storeKey = readStoreKeyQuietly();
    QJsonObject config;
    config.insert(QStringLiteral("dataDir"), dataDirectory);
    config.insert(QStringLiteral("cacheDir"), cacheDirectory);
    if (storeKey.state == StoreKeyState::Available) {
        config.insert(QStringLiteral("storeKey"), storeKey.key);
    }
    QByteArray configJson = QJsonDocument(config).toJson(QJsonDocument::Compact);
    XmCore *core = xm_core_new(configJson.constData());
    configJson.fill('\0');
    storeKey.key.fill(QChar('0'));
    if (!core) {
        qWarning("xmatic: push wake-up could not start the core");
        return 0;
    }
    xm_core_set_callback(core, &deliver, nullptr);

    AppSettings settings;
    const QString generic = QCoreApplication::translate("PushWake", "New message");
    QJsonObject gateway;
    gateway.insert(QStringLiteral("mode"), settings.pushGatewayMode());
    gateway.insert(QStringLiteral("gateway"), settings.pushGateway());
    const quint64 picked = sendCommand(core, 1, QStringLiteral("push.gateway"), gateway);
    const quint64 wake = 2;

    QTimer drain;
    QObject::connect(&drain, &QTimer::timeout, &app, [&]() {
        QStringList lines;
        {
            QMutexLocker locked(&inboxLock);
            lines.swap(inbox);
        }
        for (const QString &line : lines) {
            const QJsonObject message = QJsonDocument::fromJson(line.toUtf8()).object();
            const QString type = message.value(QStringLiteral("type")).toString();
            const QJsonObject data = message.value(QStringLiteral("data")).toObject();
            if (type == QLatin1String("reply")) {
                const quint64 id = quint64(message.value(QStringLiteral("id")).toDouble());
                if (id == picked) {
                    // The name first: Foghorn gives each call a few seconds.
                    sendCommand(core, wake, QStringLiteral("push.wake"));
                    sendCommand(core, 3, QStringLiteral("session.restore"));
                } else if (id == wake) {
                    app.quit();
                }
                continue;
            }
            const QString name = message.value(QStringLiteral("event")).toString();
            if (name == QLatin1String("push.banner")) {
                const QString roomId = data.value(QStringLiteral("roomId")).toString();
                if (data.value(QStringLiteral("generic")).toBool() || roomId.isEmpty()) {
                    publishBanner(dataDirectory, QStringLiteral("*"), QString(),
                                  generic, generic, false);
                    continue;
                }
                const bool preview = settings.notificationPreview();
                const QString room = data.value(QStringLiteral("roomName")).toString();
                const QString body = MatrixBridge::previewLine(
                    data.value(QStringLiteral("previewKind")).toString(),
                    data.value(QStringLiteral("previewText")).toString());
                publishBanner(dataDirectory, roomId, roomId,
                              preview && !room.isEmpty() ? room : generic,
                              preview && !body.isEmpty() ? body : generic,
                              data.value(QStringLiteral("noisy")).toBool());
            } else if (name == QLatin1String("push.state")
                       && data.value(QStringLiteral("state")).toString()
                              == QLatin1String("unregistered")) {
                qInfo("xmatic: the distributor unregistered this app");
                publishPushNotice(dataDirectory);
            } else if (name == QLatin1String("core.log")) {
                qInfo("xmatic: push wake %s: %s",
                      qPrintable(data.value(QStringLiteral("level")).toString()),
                      qPrintable(data.value(QStringLiteral("message")).toString()));
            }
        }
    });
    drain.start(50);

    // Yield if the app starts.
    bool yielded = false;
    QTimer yieldCheck;
    QObject::connect(&yieldCheck, &QTimer::timeout, &app, [&]() {
        // The app deletes it.
        if (!yielded
            && QFile::exists(dataDirectory + QStringLiteral("/")
                             + QStringLiteral(XMATIC_WAKE_YIELD_FILE))) {
            qInfo("xmatic: the app is starting; leaving the store to it");
            yielded = true;
            sendCommand(core, 4, QStringLiteral("push.yield"));
        }
    });
    yieldCheck.start(250);

    QTimer::singleShot(WakeLimitMs, &app, [&]() {
        qWarning("xmatic: push wake-up ran out of time");
        app.quit();
    });

    app.exec();
    xm_core_set_callback(core, nullptr, nullptr);
    xm_core_free(core);
    return 0;
}

namespace {

/// The sandbox's xdg-dbus-proxy handles pipelined SASL from Sailfish OS 5.1 on.
bool proxyHandlesPipelinedAuth()
{
    QFile release(QStringLiteral("/etc/os-release"));
    if (!release.open(QIODevice::ReadOnly)) {
        return false;
    }
    for (const QByteArray &line : release.readAll().split('\n')) {
        if (!line.startsWith("VERSION_ID=")) {
            continue;
        }
        const QList<QByteArray> parts = line.mid(11).replace('"', "").split('.');
        bool majorRead = false;
        bool minorRead = false;
        const int major = parts.value(0).toInt(&majorRead);
        const int minor = parts.value(1).toInt(&minorRead);
        if (!majorRead) {
            return false;
        }
        return major > 5 || (major == 5 && minorRead && minor >= 1);
    }
    return false;
}

} // namespace

void pushPrelude(const char *argv0)
{
    if (proxyHandlesPipelinedAuth()) {
        return;
    }
    // zbus skips pipelined SASL under FLATPAK_ID; older proxies drop it. Always set,
    // since the push page uses the bus while push is off.
    qputenv("FLATPAK_ID", QFileInfo(QString::fromLocal8Bit(argv0)).fileName().toLocal8Bit());
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
    publishBanner(dataDirectory, QStringLiteral("notice"), QString(),
                  QStringLiteral("xmatic"),
                  QCoreApplication::translate(
                      "PushWake",
                      "The push distributor dropped xmatic. Turn push back on under Account."),
                  false);
}
