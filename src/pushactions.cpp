#include "pushactions.h"

#include <QCoreApplication>
#include <QStringList>

#include "appsettings.h"
#include "pushbanners.h"

namespace {

/// `push.notify` at once; more wait, one per room.
const int MaxFetches = 2;
/// Rooms waiting for a slot.
const int MaxQueued = 16;
/// A banner tap whose room never shows up is dropped.
const qint64 PendingLimitMs = 30000;

/// A woken process's room mark under the salt it was made with.
QString tagged(const QString &salt, const QString &mark)
{
    return salt + QLatin1Char(':') + mark;
}

} // namespace

PushActions::Banner PushActions::decide(const QVariantMap &notification)
{
    Banner banner;
    // Low priority keeps its badge and never notifies, as in the room list.
    banner.show = !notification.value(QStringLiteral("roomId")).toString().isEmpty()
            && !notification.value(QStringLiteral("lowPriority")).toBool();
    // The push rules decided the sound; not judged a second time.
    banner.audible = banner.show && notification.value(QStringLiteral("noisy")).toBool();
    return banner;
}

PushActions::PushActions(const QString &dataDirectory, AppSettings *settings, QObject *parent)
    : QObject(parent)
    , m_dataDirectory(dataDirectory)
    , m_settings(settings)
{
    m_clock.start();
}

void PushActions::refresh()
{
    // Every time: an old switch whose carry-over failed must not read as off.
    QJsonObject arguments;
    arguments.insert(QStringLiteral("legacyWanted"), m_settings && m_settings->legacyPushWanted());
    emit commandReady(QStringLiteral("push.status"), arguments);
}

void PushActions::enable()
{
    emit refused(QString());
    emit commandReady(QStringLiteral("push.enable"), QJsonObject());
}

void PushActions::disable()
{
    if (m_settings) {
        m_settings->forgetLegacyPush();
    }
    emit commandReady(QStringLiteral("push.disable"), QJsonObject());
}

bool PushActions::setGateway(const QString &mode, const QString &gateway, bool enable)
{
    const QString trimmed = gateway.trimmed();
    if (mode == QLatin1String("other")
        && !trimmed.startsWith(QLatin1String("https://"), Qt::CaseInsensitive)) {
        emit refused(QCoreApplication::translate("MatrixBridge", "The gateway must be an https URL."));
        return false;
    }
    if (m_settings) {
        m_settings->setPushGateway(mode, trimmed);
        // No pick is the page's "off": the old switch goes with it.
        if (mode.isEmpty()) {
            m_settings->forgetLegacyPush();
        }
    }
    sendGateway(enable);
    return true;
}

void PushActions::selectDistributor(const QString &distributor, bool enable)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("distributor"), distributor);
    arguments.insert(QStringLiteral("enable"), enable);
    emit commandReady(QStringLiteral("push.distributor"), arguments);
}

void PushActions::sendGateway(bool enable)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("enable"), enable);
    arguments.insert(QStringLiteral("mode"), m_settings ? m_settings->pushGatewayMode() : QString());
    if (m_settings) {
        arguments.insert(QStringLiteral("gateway"), m_settings->pushGateway());
    }
    emit commandReady(QStringLiteral("push.gateway"), arguments);
}

void PushActions::start()
{
    sendGateway(false);
}

void PushActions::closeBanners()
{
    const ClosedPushBanners closed = ::closePushBanners(m_dataDirectory);
    if (closed.keys.isEmpty()) {
        return;
    }
    // Rooms are marks under the record's salt; each record had its own.
    for (auto it = closed.announced.constBegin(); it != closed.announced.constEnd(); ++it) {
        m_wakeAnnounced.insert(tagged(closed.salt, it.key()), it.value());
    }
    for (auto it = closed.keys.constBegin(); it != closed.keys.constEnd(); ++it) {
        m_wakeKeys.insert(it.key(), tagged(closed.salt, it.value()));
    }
}

void PushActions::openBanner(const QString &key)
{
    if (key.isEmpty()) {
        return;
    }
    if (!m_wakeKeys.contains(key)) {
        closeBanners();
    }
    const QString wanted = m_wakeKeys.value(key);
    if (wanted.isEmpty()) {
        return;
    }
    const QString salt = wanted.section(QLatin1Char(':'), 0, 0);
    for (const QString &roomId : m_rooms) {
        if (tagged(salt, pushRoomMark(salt, roomId)) == wanted) {
            emit bannerRoomReady(roomId);
            return;
        }
    }
    // Before the room list: answered from `noteRoom`.
    m_pendingMark = wanted;
    m_pendingSince = m_clock.elapsed();
}

void PushActions::noteRoom(const QString &roomId)
{
    m_rooms.insert(roomId);
    if (m_pendingMark.isEmpty()) {
        return;
    }
    if (m_clock.elapsed() - m_pendingSince > PendingLimitMs) {
        m_pendingMark.clear();
        return;
    }
    const QString salt = m_pendingMark.section(QLatin1Char(':'), 0, 0);
    if (tagged(salt, pushRoomMark(salt, roomId)) == m_pendingMark) {
        m_pendingMark.clear();
        emit bannerRoomReady(roomId);
    }
}

bool PushActions::syncBannerCovered(const QString &roomId, qint64 timestamp) const
{
    if (timestamp <= 0) {
        return false;
    }
    qint64 announced = m_pushed.value(roomId);
    if (!m_wakeAnnounced.isEmpty()) {
        QSet<QString> salts;
        for (auto it = m_wakeKeys.constBegin(); it != m_wakeKeys.constEnd(); ++it) {
            salts.insert(it.value().section(QLatin1Char(':'), 0, 0));
        }
        for (const QString &salt : salts) {
            announced = qMax(announced,
                             m_wakeAnnounced.value(tagged(salt, pushRoomMark(salt, roomId))));
        }
    }
    // Only the message a push announced; a later one is the sync's to announce.
    return announced >= timestamp;
}

void PushActions::noteSyncBanner(const QString &roomId, qint64 timestamp)
{
    m_synced.insert(roomId, qMax(timestamp, m_synced.value(roomId)));
}

void PushActions::forget()
{
    ::closePushBanners(m_dataDirectory);
    // The next account picks its own gateway; the core drops its copy too.
    if (m_settings) {
        m_settings->setPushGateway(QString(), QString());
    }
    m_fetching.clear();
    m_queued.clear();
    m_pushed.clear();
    m_synced.clear();
    m_wakeAnnounced.clear();
    m_wakeKeys.clear();
    m_pendingMark.clear();
    m_rooms.clear();
}

bool PushActions::handleEvent(const QString &name, const QJsonObject &data)
{
    if (name == QLatin1String("push.state")) {
        // An error belongs to the state that carried it.
        m_status = data.toVariantMap();
        const QString state = m_status.value(QStringLiteral("state")).toString();
        // Bus names of what is installed, never the endpoint: that is a secret.
        qInfo("xmatic: push state -> %s (%d distributor(s))", qPrintable(state),
              m_status.value(QStringLiteral("distributors")).toList().count());
        const QString error = m_status.value(QStringLiteral("error")).toString();
        if (!error.isEmpty()) {
            emit failed(error);
        }
        if (state == QLatin1String("unregistered")) {
            publishPushNotice(m_dataDirectory);
        }
        emit statusChanged();
        return true;
    }
    if (name == QLatin1String("push.message")) {
        const QString roomId = data.value(QStringLiteral("roomId")).toString();
        const QString eventId = data.value(QStringLiteral("eventId")).toString();
        // The running sync brings the message and its banner itself.
        const bool wanted = data.value(QStringLiteral("fetch")).toBool(true);
        qInfo("xmatic: push received (matrix=%d, fetch=%d)", roomId.isEmpty() ? 0 : 1,
              wanted ? 1 : 0);
        if (wanted && !roomId.isEmpty() && !eventId.isEmpty()) {
            fetch(roomId, eventId);
        }
        return true;
    }
    return false;
}

void PushActions::fetch(const QString &roomId, const QString &eventId)
{
    const bool roomBusy = !m_fetching.keys(roomId).isEmpty();
    if (roomBusy || m_fetching.size() >= MaxFetches) {
        // Pushes name rooms the sender chose; past the bound, the sync brings them.
        if (m_queued.contains(roomId) || m_queued.size() < MaxQueued) {
            m_queued.insert(roomId, eventId);
        }
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("eventId"), eventId);
    emit commandReady(QStringLiteral("push.notify"), arguments);
}

void PushActions::fetchQueued()
{
    const QStringList rooms = m_queued.keys();
    for (const QString &roomId : rooms) {
        if (m_fetching.size() >= MaxFetches) {
            return;
        }
        if (!m_fetching.keys(roomId).isEmpty()) {
            continue;
        }
        fetch(roomId, m_queued.take(roomId));
    }
}

void PushActions::finishFetch(quint64 id)
{
    if (m_fetching.remove(id) > 0) {
        fetchQueued();
    }
}

void PushActions::sent(quint64 id, const QString &command, const QJsonObject &arguments)
{
    if (id != 0 && command == QLatin1String("push.notify")) {
        m_fetching.insert(id, arguments.value(QStringLiteral("roomId")).toString());
    }
}

bool PushActions::deliver(quint64 id, const QString &command, const QJsonObject &data)
{
    if (command == QLatin1String("push.notify") || m_fetching.contains(id)) {
        finishFetch(id);
        if (data.value(QStringLiteral("silent")).toBool()) {
            return true;
        }
        const QVariantMap notification = data.toVariantMap();
        const Banner banner = decide(notification);
        if (!banner.show) {
            return true;
        }
        const QString roomId = notification.value(QStringLiteral("roomId")).toString();
        const qint64 timestamp = qint64(notification.value(QStringLiteral("timestamp")).toDouble());
        // The sync's banner already said it.
        if (timestamp > 0 && m_synced.value(roomId) >= timestamp) {
            return true;
        }
        m_pushed.insert(roomId, qMax(timestamp, m_pushed.value(roomId)));
        emit announce(notification, banner.audible);
        return true;
    }
    if (command == QLatin1String("push.gateway")) {
        // Only at start: later it could restart a connector being switched off.
        if (!m_statusAsked) {
            m_statusAsked = true;
            QJsonObject arguments;
            // Quiet: no bus while push is off.
            arguments.insert(QStringLiteral("quiet"), true);
            arguments.insert(QStringLiteral("legacyWanted"),
                             m_settings && m_settings->legacyPushWanted());
            emit commandReady(QStringLiteral("push.status"), arguments);
        }
        return true;
    }
    if (command == QLatin1String("push.status")) {
        // The old switch is spent once its file is gone.
        if (m_settings && data.contains(QStringLiteral("legacy"))
            && !data.value(QStringLiteral("legacy")).toBool()) {
            m_settings->forgetLegacyPush();
        }
        return true;
    }
    return command.startsWith(QLatin1String("push."));
}

bool PushActions::reportFailure(quint64 id, const QString &command, const QString &error)
{
    if (command != QLatin1String("push.notify") && !m_fetching.contains(id)) {
        return false;
    }
    finishFetch(id);
    qInfo("xmatic: push not fetched: %s", qPrintable(error.left(120)));
    return true;
}

void PushActions::abandon(quint64 id)
{
    finishFetch(id);
}
