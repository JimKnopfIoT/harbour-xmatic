#ifndef BANLIST_H
#define BANLIST_H

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QString>
#include <QVariantList>

/// `matrix.bans`: one room's ban list, and banning by address. No diff follows
/// a ban, so each answer is written into the list here.
class BanList : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString roomId READ roomId NOTIFY changed)
    Q_PROPERTY(QVariantList entries READ entries NOTIFY changed)
    Q_PROPERTY(bool canBan READ canBan NOTIFY changed)
    Q_PROPERTY(bool loaded READ loaded NOTIFY changed)
    Q_PROPERTY(bool loading READ loading NOTIFY changed)
    Q_PROPERTY(QString loadError READ loadError NOTIFY changed)
    /// A ban or lift is waiting for its answer.
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)

public:
    explicit BanList(QObject *parent = nullptr);

    QString roomId() const { return m_roomId; }
    QVariantList entries() const { return m_entries; }
    bool canBan() const { return m_canBan; }
    bool loaded() const { return m_loaded; }
    bool loading() const { return m_loading; }
    QString loadError() const { return m_loadError; }
    bool busy() const { return !m_actions.isEmpty(); }

    Q_INVOKABLE void load(const QString &roomId);
    /// `reason` may be empty. The core checks the address and refuses oneself.
    Q_INVOKABLE void ban(const QString &roomId, const QString &userId,
                         const QString &reason = QString());
    Q_INVOKABLE void unban(const QString &roomId, const QString &userId);

    void sent(quint64 id, const QString &command, const QJsonObject &arguments);
    bool deliver(quint64 id, const QJsonObject &data);
    bool reportFailure(quint64 id, const QString &error);
    /// Sign-out: nothing of the last account's rooms stays.
    void clear();

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void changed();
    void busyChanged();
    void banned(const QString &roomId, const QString &userId);
    void unbanned(const QString &roomId, const QString &userId);
    /// `action` is "ban" or "unban"; `message` is ready to show.
    void failed(const QString &action, const QString &roomId, const QString &message);

private:
    struct Pending {
        QString command;
        QString roomId;
        QString userId;
    };

    static QString shown(const QString &error);
    void removeEntry(const QString &userId);

    QString m_roomId;
    QVariantList m_entries;
    bool m_canBan = false;
    bool m_loaded = false;
    bool m_loading = false;
    QString m_loadError;
    /// The newest list request; older ones are answered and dropped.
    quint64 m_loadId = 0;
    QSet<quint64> m_loads;
    QHash<quint64, Pending> m_actions;
};

#endif // BANLIST_H
