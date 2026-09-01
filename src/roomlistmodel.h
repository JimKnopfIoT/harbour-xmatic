#ifndef ROOMLISTMODEL_H
#define ROOMLISTMODEL_H

#include "difflistmodel.h"

/// The room list. Order, unread counts and filtering all come from the sync
/// service in the core, so this only names the roles.
class RoomListModel : public DiffListModel
{
    Q_OBJECT

    /// Totals for the cover: in how many rooms is something new, and how many
    /// messages is that. Spaces, invitations and muted rooms stay out of both
    /// — a muted room asked not to be advertised.
    Q_PROPERTY(int unreadRooms READ unreadRooms NOTIFY unreadTotalsChanged)
    Q_PROPERTY(int unreadMessages READ unreadMessages NOTIFY unreadTotalsChanged)
    /// Whether at least one of the counted rooms sat at the edge of what a
    /// sync carries, so `unreadMessages` is a floor rather than a total. The
    /// cover marks it the same way the row's badge does; the two must not
    /// disagree about the same number.
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

    /// Writes a room's notification mode ("default", "all", "mentions",
    /// "mute") into the row that carries it.
    ///
    /// The core cannot push this: the mode changes the account's push rules,
    /// and the SDK's room list only emits a diff for changes it considers
    /// notable, which that is not. Without this the row keeps the state it
    /// had when it last arrived, so the marker never appears and the menu
    /// keeps offering the action that was just carried out.
    void setNotifyMode(const QString &roomId, const QString &mode);
    /// Clears a room's unread counters after it was marked read from the list.
    /// A receipt does not always bring a diff with it, and the row would keep
    /// its badge until something else touched the room.
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
