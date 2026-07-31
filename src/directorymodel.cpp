#include "directorymodel.h"

DirectoryModel::DirectoryModel(QObject *parent)
    : DiffListModel(parent)
{
}

QHash<int, QByteArray> DirectoryModel::roleNames() const
{
    QHash<int, QByteArray> names;
    names.insert(IdRole, "id");
    names.insert(NameRole, "name");
    names.insert(TopicRole, "topic");
    names.insert(AliasRole, "alias");
    names.insert(MembersRole, "members");
    names.insert(AvatarRole, "avatar");
    names.insert(CanJoinRole, "canJoin");
    return names;
}

QVariant DirectoryModel::valueFor(const QJsonObject &row, int role) const
{
    if (role == NameRole) {
        // A nameless room shows its alias, and failing that its id, rather
        // than an empty row.
        const QString name = row.value(QStringLiteral("name")).toString();
        if (!name.isEmpty()) {
            return name;
        }
        const QString alias = row.value(QStringLiteral("alias")).toString();
        return alias.isEmpty() ? row.value(QStringLiteral("id")).toString() : alias;
    }
    return DiffListModel::valueFor(row, role);
}
