#ifndef MEMBERMODEL_H
#define MEMBERMODEL_H

#include "difflistmodel.h"

/// The members of one room. The core reads them in a single pass and ships the
/// whole list as a reset; ordering (most powerful first, then by name) comes
/// from there. `membership` and `power` are carried for the actions a member
/// row will grow later (verify, direct message, remove).
class MemberModel : public DiffListModel
{
    Q_OBJECT

public:
    enum Role {
        UserIdRole = Qt::UserRole + 1,
        DisplayNameRole,
        MembershipRole,
        PowerRole,
        IsSelfRole,
        CanRemoveRole,
        AvatarRole,
    };

    explicit MemberModel(QObject *parent = nullptr);

    QHash<int, QByteArray> roleNames() const override;

    /// Drops the row of one user after a successful removal. The local store
    /// only learns of the kick on a later sync, so reloading the list right
    /// away would still show the member.
    void removeUser(const QString &userId);

    /// Rewrites one user's power level after a role change. Order unchanged;
    /// the next load sorts fresh.
    void setPower(const QString &userId, int power);

protected:
    QVariant valueFor(const QJsonObject &row, int role) const override;
};

#endif // MEMBERMODEL_H
