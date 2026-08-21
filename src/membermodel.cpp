#include "membermodel.h"

MemberModel::MemberModel(QObject *parent)
    : DiffListModel(parent)
{
}

QHash<int, QByteArray> MemberModel::roleNames() const
{
    QHash<int, QByteArray> names;
    names.insert(UserIdRole, "userId");
    names.insert(DisplayNameRole, "displayName");
    names.insert(MembershipRole, "membership");
    names.insert(PowerRole, "power");
    names.insert(IsSelfRole, "isSelf");
    names.insert(CanRemoveRole, "canRemove");
    names.insert(AvatarRole, "avatar");
    return names;
}

void MemberModel::removeUser(const QString &userId)
{
    for (int i = 0; i < rows().count(); ++i) {
        if (rows().at(i).value(QStringLiteral("userId")).toString() == userId) {
            QJsonObject operation;
            operation.insert(QStringLiteral("op"), QStringLiteral("remove"));
            operation.insert(QStringLiteral("index"), i);
            applyOperations(QJsonArray { operation });
            return;
        }
    }
}

void MemberModel::setPower(const QString &userId, int power)
{
    for (int i = 0; i < rows().count(); ++i) {
        if (rows().at(i).value(QStringLiteral("userId")).toString() == userId) {
            QJsonObject row = rows().at(i);
            row.insert(QStringLiteral("power"), power);
            QJsonObject operation;
            operation.insert(QStringLiteral("op"), QStringLiteral("set"));
            operation.insert(QStringLiteral("index"), i);
            operation.insert(QStringLiteral("value"), row);
            applyOperations(QJsonArray { operation });
            return;
        }
    }
}

QVariant MemberModel::valueFor(const QJsonObject &row, int role) const
{
    if (role == DisplayNameRole) {
        // A member without a display name is shown by their address rather
        // than an empty line.
        const QString name = row.value(QStringLiteral("displayName")).toString();
        return name.isEmpty() ? row.value(QStringLiteral("userId")).toString() : name;
    }
    return DiffListModel::valueFor(row, role);
}
