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

public:
    enum Role {
        IdRole = Qt::UserRole + 1,
        NameRole,
        UnreadRole,
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

    int unreadRooms() const { return m_unreadRooms; }
    int unreadMessages() const { return m_unreadMessages; }

signals:
    void unreadTotalsChanged();

protected:
    QVariant valueFor(const QJsonObject &row, int role) const override;

private:
    void recountUnread();

    int m_unreadRooms = 0;
    int m_unreadMessages = 0;
};

#endif // ROOMLISTMODEL_H
