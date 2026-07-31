#ifndef DIRECTORYMODEL_H
#define DIRECTORYMODEL_H

#include "difflistmodel.h"

/// Results of a public-room-directory search. Paging and ordering come from
/// the core's search task; this only names the roles.
class DirectoryModel : public DiffListModel
{
    Q_OBJECT

public:
    enum Role {
        IdRole = Qt::UserRole + 1,
        NameRole,
        TopicRole,
        AliasRole,
        MembersRole,
        AvatarRole,
        CanJoinRole,
    };

    explicit DirectoryModel(QObject *parent = nullptr);

    QHash<int, QByteArray> roleNames() const override;

protected:
    QVariant valueFor(const QJsonObject &row, int role) const override;
};

#endif // DIRECTORYMODEL_H
