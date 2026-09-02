#ifndef MEMBERMODEL_H
#define MEMBERMODEL_H

#include "difflistmodel.h"

/// The members of one room, shipped as one reset. Ordering comes from the core;
/// `membership` and `power` carry what the row's actions need.
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

    /// Drops one user's row after a successful removal: the store learns of the
    /// kick on a later sync, so a reload would still show the member.
    void removeUser(const QString &userId);

    /// Rewrites one user's power level after a role change. Order unchanged;
    /// the next load sorts fresh.
    void setPower(const QString &userId, int power);

protected:
    QVariant valueFor(const QJsonObject &row, int role) const override;
};

#endif // MEMBERMODEL_H
