#ifndef BEHAVIOURSETTINGS_H
#define BEHAVIOURSETTINGS_H

#include <QObject>
#include <QString>
#include <QVariant>

/// Which room events the timeline shows, and how the room and space lists sort.
/// Events off by default: a history is read for its messages, not for who came and went.
class BehaviourSettings : public QObject
{
    Q_OBJECT

    Q_PROPERTY(bool showJoinLeaves READ showJoinLeaves WRITE setShowJoinLeaves NOTIFY changed)
    Q_PROPERTY(bool showDisplayNameChanges READ showDisplayNameChanges
               WRITE setShowDisplayNameChanges NOTIFY changed)
    Q_PROPERTY(bool showAvatarChanges READ showAvatarChanges WRITE setShowAvatarChanges
               NOTIFY changed)
    /// "activity" (the SDK's order) or "name".
    Q_PROPERTY(QString roomOrder READ roomOrder WRITE setRoomOrder NOTIFY changed)
    Q_PROPERTY(bool unreadFirst READ unreadFirst WRITE setUnreadFirst NOTIFY changed)
    Q_PROPERTY(QString spaceOrder READ spaceOrder WRITE setSpaceOrder NOTIFY changed)

public:
    explicit BehaviourSettings(QObject *parent = nullptr);

    bool showJoinLeaves() const { return m_showJoinLeaves; }
    bool showDisplayNameChanges() const { return m_showDisplayNameChanges; }
    bool showAvatarChanges() const { return m_showAvatarChanges; }
    QString roomOrder() const { return m_roomOrder; }
    bool unreadFirst() const { return m_unreadFirst; }
    QString spaceOrder() const { return m_spaceOrder; }

    void setShowJoinLeaves(bool shown);
    void setShowDisplayNameChanges(bool shown);
    void setShowAvatarChanges(bool shown);
    void setRoomOrder(const QString &order);
    void setUnreadFirst(bool first);
    void setSpaceOrder(const QString &order);

signals:
    void changed();

private:
    void store(const QString &key, const QVariant &value);

    bool m_showJoinLeaves = false;
    bool m_showDisplayNameChanges = false;
    bool m_showAvatarChanges = false;
    QString m_roomOrder;
    bool m_unreadFirst = false;
    QString m_spaceOrder;
};

#endif // BEHAVIOURSETTINGS_H
