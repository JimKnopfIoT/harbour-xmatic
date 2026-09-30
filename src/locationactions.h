#ifndef LOCATIONACTIONS_H
#define LOCATIONACTIONS_H

#include <QElapsedTimer>
#include <QGeoCoordinate>
#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QTimer>
#include <QVariantMap>

class AppSettings;
class QGeoPositionInfo;
class QGeoPositionInfoSource;

/// Locations, as QML reads them: `matrix.locations`. Owns the position source,
/// the running live shares and the map answers; both privacy settings are
/// enforced here, in front of the commands, not in the page that offers them.
class LocationActions : public QObject
{
    Q_OBJECT
    /// The last fix, `{lat, lon, accuracy}`, or empty while there is none.
    Q_PROPERTY(QVariantMap position READ position NOTIFY positionChanged)
    /// Why there is no fix, in words; empty while there is one or none was asked.
    Q_PROPERTY(QString positionError READ positionError NOTIFY positionChanged)
    Q_PROPERTY(bool locating READ locating NOTIFY positionChanged)
    /// Rooms this phone is sharing its live position into right now.
    Q_PROPERTY(QStringList liveRooms READ liveRooms NOTIFY liveChanged)

public:
    explicit LocationActions(AppSettings *settings, QObject *parent = nullptr);

    QVariantMap position() const { return m_position; }
    QString positionError() const { return m_positionError; }
    bool locating() const { return m_locating; }
    QStringList liveRooms() const { return m_live.keys(); }

    /// Asks the device for a fresh fix; `positionChanged` answers.
    Q_INVOKABLE void locate();
    /// Stops asking where nobody waits for a fix any more.
    Q_INVOKABLE void release();
    /// The last fix into the open room, once.
    Q_INVOKABLE bool sendCurrent();
    Q_INVOKABLE bool startLive(const QString &roomId, int minutes);
    /// Also for a share this run never started: one left over from before a restart.
    Q_INVOKABLE void stopLive(const QString &roomId);
    Q_INVOKABLE bool isLive(const QString &roomId) const { return m_live.contains(roomId); }
    Q_INVOKABLE qint64 liveUntil(const QString &roomId) const { return m_live.value(roomId); }

    /// The map for a point, or an empty map while it is fetched or not allowed.
    /// `encrypted` counts unknown as encrypted - the gate's safe direction.
    Q_INVOKABLE QVariantMap map(double lat, double lon, bool encrypted);
    Q_INVOKABLE bool mapsAllowed(bool encrypted) const;
    /// One spelling of a point, so a card finds the answer to its own question.
    Q_INVOKABLE static QString mapKey(double lat, double lon);

    /// The bridge hands back the id each command went out under.
    void sent(quint64 id, const QString &command, const QJsonObject &arguments);
    /// A reply that belongs here; false when the id is not ours.
    bool deliver(quint64 id, const QString &command, const QJsonObject &data);
    /// A refusal; true when it must not reach the banner.
    bool reportFailure(quint64 id, const QString &command);

    /// Watchdog gave up on `id`: a started share is stopped, after the start.
    void abandon(quint64 id);
    /// Sign-out and media wipe: the tiles are gone, so are the paths to them.
    void clear();
    /// Sign-out: the shares belonged to the account that is gone.
    void forgetLive();

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void positionChanged();
    void liveChanged();
    void mapReady(const QString &key, const QVariantMap &map);
    void liveFailed(const QString &roomId);

private:
    void ensureSource();
    void onPosition(const QGeoPositionInfo &info);
    void sendBeacons(bool force);
    void expire();
    void updateTracking();

    AppSettings *m_settings = nullptr;
    QGeoPositionInfoSource *m_source = nullptr;
    QVariantMap m_position;
    QString m_positionError;
    bool m_locating = false;
    bool m_wanted = false;

    /// Room → end of the share, ms since the epoch.
    QHash<QString, qint64> m_live;
    /// Room → where and when the last beacon went out.
    QHash<QString, QGeoCoordinate> m_lastSentAt;
    QHash<QString, qint64> m_lastSentTime;
    /// Beacons in flight → their room.
    QHash<quint64, QString> m_roomOf;
    /// Starts in flight → their room; stopped or signed out before the answer.
    QHash<quint64, QString> m_starting;
    QSet<quint64> m_cancelled;
    QSet<quint64> m_dropped;
    QTimer m_expiry;

    QHash<QString, QVariantMap> m_maps;
    QHash<QString, qint64> m_mapAsked;
    QSet<QString> m_mapFailed;
    QElapsedTimer m_clock;
};

#endif // LOCATIONACTIONS_H
