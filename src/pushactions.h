#ifndef PUSHACTIONS_H
#define PUSHACTIONS_H

#include <QElapsedTimer>
#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QVariantMap>

class AppSettings;

/// Push as QML reads it: `matrix.push`. Status, the switch, the gateway and
/// distributor picks, and the banners a push raises. Hands finished commands up;
/// the bridge sends them and hands back the ids.
class PushActions : public QObject
{
    Q_OBJECT
    /// `state`, `distributors`, `distributor`, `enabled`, `registered`, `gateway`,
    /// `gatewayMode`, `serverGateway`, `pushService`, `error`. Never the endpoint.
    Q_PROPERTY(QVariantMap status READ status NOTIFY statusChanged)
    /// Whether this device has a push address. The address itself is a secret.
    Q_PROPERTY(bool endpointReady READ endpointReady NOTIFY statusChanged)

public:
    /// One rule for the app and the woken process.
    struct Banner {
        bool show = false;
        bool audible = false;
    };
    static Banner decide(const QVariantMap &notification);

    PushActions(const QString &dataDirectory, AppSettings *settings, QObject *parent = nullptr);

    QVariantMap status() const { return m_status; }
    bool endpointReady() const { return m_status.value(QStringLiteral("enabled")).toBool(); }

    Q_INVOKABLE void refresh();
    Q_INVOKABLE void enable();
    Q_INVOKABLE void disable();
    /// Saves the pick and tells the core; false for a gateway that is not https.
    /// `enable` turns push on with it once the pick is complete.
    Q_INVOKABLE bool setGateway(const QString &mode, const QString &gateway = QString(),
                                bool enable = false);
    /// One of several installed distributors; `enable` turns push on with it.
    Q_INVOKABLE void selectDistributor(const QString &distributor, bool enable);
    /// Takes over the banners standing from before; closes those that name no room.
    Q_INVOKABLE void adoptBanners();
    /// A room's banner, replacing that room's last one. `count` its notifying events.
    Q_INVOKABLE void raiseBanner(const QString &roomId, const QString &summary,
                                 const QString &body, bool audible, int count);
    /// The room a banner's action key stands for, answered by `bannerRoomReady`
    /// once the room is known here.
    Q_INVOKABLE void openBanner(const QString &key);

    /// At start: the gateway pick first, its reply asks for the status.
    void start();
    /// Sign-out: banners and what they named go.
    void forget();

    /// A push-related event; false where the name is not ours.
    bool handleEvent(const QString &name, const QJsonObject &data);
    void sent(quint64 id, const QString &command, const QJsonObject &arguments);
    /// A reply that belongs here; false when the id is not ours.
    bool deliver(quint64 id, const QString &command, const QJsonObject &data);
    /// A refusal; true when it must not reach the banner.
    bool reportFailure(quint64 id, const QString &command, const QString &error);
    /// The watchdog gave up on `id`.
    void abandon(quint64 id);

    /// A room the room list carries; resolves a banner tap waiting for it.
    void noteRoom(const QString &roomId);
    /// A room with nothing left to notify, its latest event at `timestamp`. Without
    /// `dropped` (the count just fell) only a later event proves it: counts lag the event.
    void roomSettled(const QString &roomId, qint64 timestamp, bool dropped);
    /// Whether a push banner already announced the room's message at `timestamp`.
    bool syncBannerCovered(const QString &roomId, qint64 timestamp) const;
    /// The sync's banner announced the room's message at `timestamp`.
    void noteSyncBanner(const QString &roomId, qint64 timestamp);

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void statusChanged();
    /// A push banner the app should raise, after its own on-screen test.
    void announce(const QVariantMap &notification, bool audible);
    void bannerRoomReady(const QString &roomId);
    /// For the error log.
    void failed(const QString &message);
    /// For the app's error line; empty clears it.
    void refused(const QString &message);

private:
    void sendGateway(bool enable);
    void fetch(const QString &roomId, const QString &eventId);
    void fetchQueued();
    void finishFetch(quint64 id);
    void readBanners();
    void dropBanner(const QString &roomId);

    QString m_dataDirectory;
    AppSettings *m_settings = nullptr;
    QVariantMap m_status;
    bool m_statusAsked = false;
    QElapsedTimer m_clock;

    /// `push.notify` in flight, by id, with its room; at most `MaxFetches`.
    QHash<quint64, QString> m_fetching;
    /// The latest event per room waiting for a free slot.
    QHash<QString, QString> m_queued;
    /// Timestamp of the latest message announced, by push and by sync, per room.
    QHash<QString, qint64> m_pushed;
    QHash<QString, qint64> m_synced;

    /// The record of standing banners as last read: its salt, action key to room mark.
    QString m_bannerSalt;
    QHash<QString, QString> m_bannerKeys;
    /// What every banner seen announced, by `salt:mark`.
    QHash<QString, qint64> m_bannerAnnounced;
    /// A tap waiting for its room to appear, and since when.
    QString m_pendingMark;
    qint64 m_pendingSince = 0;
    QSet<QString> m_rooms;
};

#endif // PUSHACTIONS_H
