#ifndef ROOMLISTMODEL_H
#define ROOMLISTMODEL_H

#include "difflistmodel.h"

/// The room list. Order, unread counts and filtering all come from the sync
/// service in the core, so this only names the roles.
class RoomListModel : public DiffListModel
{
    Q_OBJECT

    /// Totals for the cover: how many rooms have something new and how many
    /// messages that is. Spaces, invitations and muted rooms stay out of both.
    Q_PROPERTY(int unreadRooms READ unreadRooms NOTIFY unreadTotalsChanged)
    Q_PROPERTY(int unreadMessages READ unreadMessages NOTIFY unreadTotalsChanged)
    /// Whether a counted room sat at the edge of what a sync carries, so the total
    /// is a floor. The cover marks it the way the row's badge does.
    Q_PROPERTY(bool unreadCapped READ unreadCapped NOTIFY unreadTotalsChanged)

public:
    enum Role {
        IdRole = Qt::UserRole + 1,
        NameRole,
        UnreadRole,
        UnreadCappedRole,
        MentionsRole,
        EncryptedRole,
        SpaceRole,
        MembershipRole,
        AvatarRole,
        TimestampRole,
        MutedRole,
        FavouriteRole,
        LowPriorityRole,
        TombstonedRole,
    };

    explicit RoomListModel(QObject *parent = nullptr);

    QHash<int, QByteArray> roleNames() const override;

    /// Writes a room's notification mode into its row: the mode is a push rule,
    /// which the SDK's room list does not count as a notable change.
    void setNotifyMode(const QString &roomId, const QString &mode);
    /// Clears a room's counters after it was marked read from the list: a receipt
    /// does not always bring a diff, and the badge would stand.
    void clearUnread(const QString &roomId);

    int unreadRooms() const { return m_unreadRooms; }
    int unreadMessages() const { return m_unreadMessages; }
    bool unreadCapped() const { return m_unreadCapped; }

signals:
    void unreadTotalsChanged();

protected:
    QVariant valueFor(const QJsonObject &row, int role) const override;

private:
    void recountUnread();

    int m_unreadRooms = 0;
    int m_unreadMessages = 0;
    bool m_unreadCapped = false;
};

#endif // ROOMLISTMODEL_H
