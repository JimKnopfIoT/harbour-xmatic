#include "pushwake.h"

#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMutex>
#include <QStandardPaths>
#include <QTimer>
#include <QVariantMap>

#include <sys/prctl.h>
#include <sys/stat.h>

#include "appsettings.h"
#include "instancelock.h"
#include "languagesettings.h"
#include "matrixbridge.h"
#include "pushactions.h"
#include "pushbanners.h"
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

bool isBase64(const QString &text)
{
    for (const QChar c : text) {
        const ushort u = c.unicode();
        const bool fits = (u >= 'A' && u <= 'Z') || (u >= 'a' && u <= 'z')
                || (u >= '0' && u <= '9') || u == '+' || u == '/' || u == '=';
        if (!fits) {
            return false;
        }
    }
    return !text.isEmpty();
}

/// The store key, read only now that a push came in, into one buffer that is
/// wiped as soon as the core has its copy. No key: the core answers "locked".
void sendUnlock(XmCore *core, quint64 id)
{
    StoreKeyResult storeKey = readStoreKeyQuietly();
    const bool usable = storeKey.state == StoreKeyState::Available && isBase64(storeKey.key);
    QByteArray payload;
    // Reserved up front: a growing buffer would leave copies behind.
    payload.reserve(96 + storeKey.key.size());
    payload.append("{\"cmd\":\"push.unlock\",\"id\":");
    payload.append(QByteArray::number(id));
    if (usable) {
        payload.append(",\"storeKey\":\"");
        for (const QChar c : storeKey.key) {
            payload.append(char(c.unicode()));
        }
        payload.append('"');
    }
    payload.append('}');
    xm_core_send(core, payload.constData());
    payload.fill('\0');
    storeKey.key.fill(QChar('0'));
    if (!usable) {
        qInfo("xmatic: push wake-up without the store key (state %d)",
              static_cast<int>(storeKey.state));
    }
}

enum class PushSetting {
    Off,
    On,
    /// Push from 0.45 or before, not yet carried over by the app.
    NotMigrated,
};

/// Leghorn's saved state, then the old connector's: nothing wakes unless push was on.
PushSetting pushSetting(const QString &dataDirectory, bool legacyWanted)
{
    QFile state(dataDirectory + QStringLiteral("/leghorn.json"));
    if (state.open(QIODevice::ReadOnly)
        && QJsonDocument::fromJson(state.readAll()).object()
               .value(QStringLiteral("enabled")).toBool()) {
        return PushSetting::On;
    }
    QFile legacy(dataDirectory + QStringLiteral("/push.json"));
    if (!legacy.open(QIODevice::ReadOnly)) {
        return PushSetting::Off;
    }
    const QJsonArray registrations = QJsonDocument::fromJson(legacy.readAll()).object()
            .value(QStringLiteral("registrations")).toArray();
    return !registrations.isEmpty() || legacyWanted ? PushSetting::NotMigrated
                                                    : PushSetting::Off;
}

QString ensureDirectory(QStandardPaths::StandardLocation location)
{
    const QString path = QStandardPaths::writableLocation(location);
    if (path.isEmpty() || !QDir().mkpath(path)) {
        return QString();
    }
    return path;
}

/// Set by `pushPrelude`: FLATPAK_ID is this process's, not a helper's.
bool flatpakIdSet = false;

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
    // Before any text is built: a banner speaks the app's language.
    LanguageSettings::applyWithDeviceLocale(&app);

    AppSettings settings;
    const PushSetting setting =
            pushSetting(QStandardPaths::writableLocation(QStandardPaths::AppDataLocation),
                        settings.legacyPushWanted());
    if (setting == PushSetting::Off) {
        qInfo("xmatic: push is off; not waking");
        return 0;
    }

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

    // The old registration is the app's to carry over; this process only says so.
    if (setting == PushSetting::NotMigrated) {
        qInfo("xmatic: push from an older version; asking for the app to be opened");
        publishPushMigrationNotice(dataDirectory);
        return 0;
    }

    // One process per store. A lock that cannot be taken is a store this
    // process stays out of; the app asks for it through the yield file.
    if (!acquireInstanceLock(dataDirectory, false)) {
        qInfo("xmatic: the store is not ours to take; not waking for a push");
        return 0;
    }

    // No store key here: it is read once a push with a valid token came in.
    QJsonObject config;
    config.insert(QStringLiteral("dataDir"), dataDirectory);
    config.insert(QStringLiteral("cacheDir"), cacheDirectory);
    const QByteArray configJson = QJsonDocument(config).toJson(QJsonDocument::Compact);
    XmCore *core = xm_core_new(configJson.constData());
    if (!core) {
        qWarning("xmatic: push wake-up could not start the core");
        return 0;
    }
    xm_core_set_callback(core, &deliver, nullptr);

    const QString generic = QCoreApplication::translate("PushWake", "New message");
    QJsonObject gateway;
    gateway.insert(QStringLiteral("mode"), settings.pushGatewayMode());
    gateway.insert(QStringLiteral("gateway"), settings.pushGateway());
    const quint64 picked = sendCommand(core, 1, QStringLiteral("push.gateway"), gateway);
    const quint64 wake = 2;
    const quint64 unlock = 3;

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
                } else if (id == wake) {
                    app.quit();
                }
                continue;
            }
            const QString name = message.value(QStringLiteral("event")).toString();
            if (name == QLatin1String("push.unlock")) {
                sendUnlock(core, unlock);
            } else if (name == QLatin1String("push.banner")) {
                const QString roomId = data.value(QStringLiteral("roomId")).toString();
                if (data.value(QStringLiteral("generic")).toBool() || roomId.isEmpty()) {
                    publishPushBanner(dataDirectory, QString(), QStringLiteral("*"),
                                      generic, generic, false, 0);
                    continue;
                }
                const QVariantMap notification = data.toVariantMap();
                const PushActions::Banner banner = PushActions::decide(notification);
                if (!banner.show) {
                    continue;
                }
                const bool preview = settings.notificationPreview();
                const QString room = data.value(QStringLiteral("roomName")).toString();
                const QString body = MatrixBridge::previewLine(
                    data.value(QStringLiteral("previewKind")).toString(),
                    data.value(QStringLiteral("previewText")).toString());
                publishPushBanner(dataDirectory, roomId, QString(),
                                  preview && !room.isEmpty() ? room : generic,
                                  preview && !body.isEmpty() ? body : generic,
                                  banner.audible,
                                  qint64(data.value(QStringLiteral("timestamp")).toDouble()));
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

    // Ends the wake; quits even if the core does not answer in time.
    bool yielded = false;
    auto yieldNow = [&]() {
        if (yielded) {
            return;
        }
        yielded = true;
        sendCommand(core, 4, QStringLiteral("push.yield"));
        QTimer::singleShot(XMATIC_WAKE_YIELD_MS, &app, &QCoreApplication::quit);
    };

    QTimer yieldCheck;
    QObject::connect(&yieldCheck, &QTimer::timeout, &app, [&]() {
        // The app deletes it.
        if (!yielded
            && QFile::exists(dataDirectory + QStringLiteral("/")
                             + QStringLiteral(XMATIC_WAKE_YIELD_FILE))) {
            qInfo("xmatic: the app is starting; leaving the store to it");
            yieldNow();
        }
    });
    yieldCheck.start(250);

    QTimer::singleShot(WakeLimitMs, &app, [&]() {
        qWarning("xmatic: push wake-up ran out of time");
        yieldNow();
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
    // zbus skips pipelined SASL under FLATPAK_ID; older proxies drop it. Set for
    // this process, since the push page uses the bus while push is off.
    qputenv("FLATPAK_ID", QFileInfo(QString::fromLocal8Bit(argv0)).fileName().toLocal8Bit());
    flatpakIdSet = true;
}

QProcessEnvironment helperEnvironment()
{
    QProcessEnvironment environment = QProcessEnvironment::systemEnvironment();
    if (flatpakIdSet) {
        environment.remove(QStringLiteral("FLATPAK_ID"));
    }
    return environment;
}
