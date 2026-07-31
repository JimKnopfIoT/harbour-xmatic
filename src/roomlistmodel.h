#ifndef ROOMLISTMODEL_H
#define ROOMLISTMODEL_H

#include "difflistmodel.h"

/// The room list. Order, unread counts and filtering all come from the sync
/// service in the core, so this only names the roles.
class RoomListModel : public DiffListModel
{
    Q_OBJECT

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
    };

    explicit RoomListModel(QObject *parent = nullptr);

    QHash<int, QByteArray> roleNames() const override;

    /// Writes a room's mute state into the row that carries it.
    ///
    /// The core cannot push this: muting changes the account's push rules, and
    /// the SDK's room list only emits a diff for changes it considers notable,
    /// which that is not. Without this the row keeps the state it had when it
    /// last arrived, so the marker never appears and the menu keeps offering
    /// the action that was just carried out.
    void setMuted(const QString &roomId, bool muted);

protected:
    QVariant valueFor(const QJsonObject &row, int role) const override;
};

#endif // ROOMLISTMODEL_H
