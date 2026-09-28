#include "locationactions.h"

#include "appsettings.h"

#include <QDateTime>
#include <QGeoPositionInfo>
#include <QGeoPositionInfoSource>
#include <QJsonArray>

/// A beacon goes out when the phone moved this far, or at the latest after the
/// heartbeat; never more often than the floor.
static const double MoveMetres = 25.0;
static const qint64 BeaconFloorMs = 15 * 1000;
static const qint64 HeartbeatMs = 2 * 60 * 1000;
static const int UpdateIntervalMs = 10 * 1000;
/// A fix older than this is not "where I am".
static const qint64 StaleFixMs = 2 * 60 * 1000;
static const qint64 MapAskTimeoutMs = 60 * 1000;

LocationActions::LocationActions(AppSettings *settings, QObject *parent)
    : QObject(parent)
    , m_settings(settings)
{
    m_clock.start();
    m_expiry.setInterval(30 * 1000);
    connect(&m_expiry, &QTimer::timeout, this, [this]() {
        expire();
        sendBeacons(false);
    });
    // Switched off while shares run: they end now, not at their timeout.
    if (m_settings) {
        connect(m_settings, &AppSettings::locationSharingChanged, this, [this]() {
            if (!m_settings->locationSharing()) {
                for (const QString &room : m_live.keys()) {
                    stopLive(room);
                }
            }
        });
    }
}

void LocationActions::ensureSource()
{
    if (m_source) {
        return;
    }
    m_source = QGeoPositionInfoSource::createDefaultSource(this);
    if (!m_source) {
        return;
    }
    m_source->setPreferredPositioningMethods(QGeoPositionInfoSource::AllPositioningMethods);
    connect(m_source, &QGeoPositionInfoSource::positionUpdated,
            this, &LocationActions::onPosition);
    connect(m_source,
            static_cast<void (QGeoPositionInfoSource::*)(QGeoPositionInfoSource::Error)>(
                &QGeoPositionInfoSource::error),
            this, [this](QGeoPositionInfoSource::Error error) {
                qWarning("xmatic: positioning error %d", static_cast<int>(error));
                m_locating = false;
                m_positionError = error == QGeoPositionInfoSource::AccessError
                        ? tr("Location is switched off or not allowed.")
                        : tr("The position could not be determined.");
                emit positionChanged();
            });
    connect(m_source, &QGeoPositionInfoSource::updateTimeout, this, [this]() {
        if (m_position.isEmpty()) {
            m_locating = false;
            m_positionError = tr("No position yet. Outdoors it usually comes faster.");
            emit positionChanged();
        }
    });
}

void LocationActions::locate()
{
    if (!m_settings || !m_settings->locationSharing()) {
        return;
    }
    ensureSource();
    if (!m_source) {
        m_positionError = tr("This device offers no positioning.");
        emit positionChanged();
        return;
    }
    // A fix from a moment ago stands; an old one does not.
    const qint64 taken = m_position.value(QStringLiteral("taken")).toLongLong();
    if (QDateTime::currentMSecsSinceEpoch() - taken > StaleFixMs) {
        m_position.clear();
    }
    m_positionError.clear();
    m_locating = true;
    m_wanted = true;
    emit positionChanged();
    updateTracking();
}

void LocationActions::release()
{
    m_wanted = false;
    m_locating = false;
    emit positionChanged();
    updateTracking();
}

void LocationActions::updateTracking()
{
    if (!m_source) {
        return;
    }
    if (m_wanted || !m_live.isEmpty()) {
        m_source->setUpdateInterval(UpdateIntervalMs);
        m_source->startUpdates();
    } else {
        m_source->stopUpdates();
    }
    if (m_live.isEmpty()) {
        m_expiry.stop();
    } else if (!m_expiry.isActive()) {
        m_expiry.start();
    }
}

void LocationActions::onPosition(const QGeoPositionInfo &info)
{
    if (!info.isValid() || !info.coordinate().isValid()) {
        return;
    }
    QVariantMap position;
    position.insert(QStringLiteral("lat"), info.coordinate().latitude());
    position.insert(QStringLiteral("lon"), info.coordinate().longitude());
    if (info.hasAttribute(QGeoPositionInfo::HorizontalAccuracy)) {
        position.insert(QStringLiteral("accuracy"),
                        info.attribute(QGeoPositionInfo::HorizontalAccuracy));
    }
    position.insert(QStringLiteral("taken"), QDateTime::currentMSecsSinceEpoch());
    m_position = position;
    m_positionError.clear();
    m_locating = false;
    emit positionChanged();
    sendBeacons(false);
}

bool LocationActions::sendCurrent()
{
    if (!m_settings || !m_settings->locationSharing() || m_position.isEmpty()) {
        return false;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("lat"), m_position.value(QStringLiteral("lat")).toDouble());
    arguments.insert(QStringLiteral("lon"), m_position.value(QStringLiteral("lon")).toDouble());
    if (m_position.contains(QStringLiteral("accuracy"))) {
        arguments.insert(QStringLiteral("accuracy"),
                         m_position.value(QStringLiteral("accuracy")).toDouble());
    }
    emit commandReady(QStringLiteral("location.send"), arguments);
    return true;
}

bool LocationActions::startLive(const QString &roomId, int minutes)
{
    if (!m_settings || !m_settings->locationSharing() || roomId.isEmpty() || minutes <= 0) {
        return false;
    }
    ensureSource();
    if (!m_source) {
        return false;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("durationMs"), static_cast<double>(minutes) * 60 * 1000);
    emit commandReady(QStringLiteral("location.liveStart"), arguments);
    return true;
}

void LocationActions::stopLive(const QString &roomId)
{
    if (roomId.isEmpty()) {
        return;
    }
    // Gone here at once: the positions stop now, whatever the server answers.
    if (m_live.remove(roomId) > 0) {
        m_lastSentAt.remove(roomId);
        m_lastSentTime.remove(roomId);
        emit liveChanged();
        updateTracking();
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    emit commandReady(QStringLiteral("location.liveStop"), arguments);
}

void LocationActions::sendBeacons(bool force)
{
    if (m_live.isEmpty() || m_position.isEmpty()) {
        return;
    }
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    if (now - m_position.value(QStringLiteral("taken")).toLongLong() > StaleFixMs) {
        return;
    }
    const QGeoCoordinate here(m_position.value(QStringLiteral("lat")).toDouble(),
                              m_position.value(QStringLiteral("lon")).toDouble());
    for (auto it = m_live.constBegin(); it != m_live.constEnd(); ++it) {
        const QString &room = it.key();
        const qint64 last = m_lastSentTime.value(room, 0);
        if (!force) {
            if (now - last < BeaconFloorMs) {
                continue;
            }
            const bool moved = !m_lastSentAt.contains(room)
                    || m_lastSentAt.value(room).distanceTo(here) >= MoveMetres;
            if (!moved && now - last < HeartbeatMs) {
                continue;
            }
        }
        m_lastSentAt.insert(room, here);
        m_lastSentTime.insert(room, now);
        QJsonObject arguments;
        arguments.insert(QStringLiteral("roomId"), room);
        arguments.insert(QStringLiteral("lat"), here.latitude());
        arguments.insert(QStringLiteral("lon"), here.longitude());
        if (m_position.contains(QStringLiteral("accuracy"))) {
            arguments.insert(QStringLiteral("accuracy"),
                             m_position.value(QStringLiteral("accuracy")).toDouble());
        }
        emit commandReady(QStringLiteral("location.beacon"), arguments);
    }
}

void LocationActions::expire()
{
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    bool changed = false;
    for (const QString &room : m_live.keys()) {
        if (m_live.value(room) <= now) {
            m_live.remove(room);
            m_lastSentAt.remove(room);
            m_lastSentTime.remove(room);
            changed = true;
        }
    }
    if (changed) {
        emit liveChanged();
        updateTracking();
    }
}

bool LocationActions::mapsAllowed(bool encrypted) const
{
    if (!m_settings) {
        return false;
    }
    const QString policy = m_settings->locationMaps();
    return policy == QLatin1String("always")
            || (policy == QLatin1String("unencrypted") && !encrypted);
}

QString LocationActions::mapKey(double lat, double lon)
{
    return QString::number(lat, 'f', 6) + QLatin1Char(',') + QString::number(lon, 'f', 6);
}

QVariantMap LocationActions::map(double lat, double lon, bool encrypted)
{
    if (!mapsAllowed(encrypted)) {
        return QVariantMap();
    }
    const QString key = mapKey(lat, lon);
    const auto known = m_maps.constFind(key);
    if (known != m_maps.constEnd()) {
        return *known;
    }
    // Failed once: the card shows coordinates for this run, it does not keep asking.
    if (m_mapFailed.contains(key)) {
        return QVariantMap();
    }
    const qint64 askedAt = m_mapAsked.value(key, -1);
    if (askedAt < 0 || m_clock.elapsed() - askedAt > MapAskTimeoutMs) {
        m_mapAsked.insert(key, m_clock.elapsed());
        QJsonObject arguments;
        arguments.insert(QStringLiteral("key"), key);
        arguments.insert(QStringLiteral("lat"), lat);
        arguments.insert(QStringLiteral("lon"), lon);
        emit commandReady(QStringLiteral("location.tiles"), arguments);
    }
    return QVariantMap();
}

void LocationActions::sent(quint64 id, const QString &command, const QJsonObject &arguments)
{
    if (id == 0) {
        return;
    }
    if (command == QLatin1String("location.liveStart")
        || command == QLatin1String("location.beacon")) {
        m_roomOf.insert(id, arguments.value(QStringLiteral("roomId")).toString());
    }
}

bool LocationActions::deliver(quint64 id, const QString &command, const QJsonObject &data)
{
    if (command == QLatin1String("location.tiles")) {
        const QString key = data.value(QStringLiteral("key")).toString();
        m_mapAsked.remove(key);
        const QVariantMap map = data.toVariantMap();
        if (data.value(QStringLiteral("available")).toBool()) {
            m_maps.insert(key, map);
        } else {
            m_mapFailed.insert(key);
        }
        emit mapReady(key, map);
        return true;
    }
    if (command == QLatin1String("location.liveStart")) {
        m_roomOf.remove(id);
        const QString room = data.value(QStringLiteral("roomId")).toString();
        const qint64 until = static_cast<qint64>(data.value(QStringLiteral("until")).toDouble());
        if (room.isEmpty() || until <= QDateTime::currentMSecsSinceEpoch()) {
            return true;
        }
        m_live.insert(room, until);
        emit liveChanged();
        updateTracking();
        // The first point at once where there is a fix, else with the first one.
        sendBeacons(true);
        return true;
    }
    if (command == QLatin1String("location.beacon")) {
        m_roomOf.remove(id);
        return true;
    }
    return command == QLatin1String("location.send")
            || command == QLatin1String("location.liveStop");
}

bool LocationActions::reportFailure(quint64 id, const QString &command)
{
    const QString room = m_roomOf.take(id);
    if (command == QLatin1String("location.liveStart")) {
        emit liveFailed(room);
        return false;
    }
    // Periodic and nobody's errand; the journal keeps it, the next fix tries again.
    if (command == QLatin1String("location.beacon")) {
        m_lastSentTime.remove(room);
        return true;
    }
    return false;
}

void LocationActions::clear()
{
    m_maps.clear();
    m_mapAsked.clear();
    m_mapFailed.clear();
}

void LocationActions::forgetLive()
{
    if (m_live.isEmpty()) {
        return;
    }
    m_live.clear();
    m_lastSentAt.clear();
    m_lastSentTime.clear();
    emit liveChanged();
    updateTracking();
}
