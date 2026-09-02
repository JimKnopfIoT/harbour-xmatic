#ifndef SEARCHMODEL_H
#define SEARCHMODEL_H

#include "difflistmodel.h"

/// Search hits in one room. Nothing streams: the core answers one page per
/// command, turned into a reset or an append. Only the row storage is borrowed.
class SearchModel : public DiffListModel
{
    Q_OBJECT

public:
    enum Role {
        EventIdRole = Qt::UserRole + 1,
        SenderRole,
        SenderNameRole,
        TimestampRole,
        BodyRole,
    };

    explicit SearchModel(QObject *parent = nullptr);

    QHash<int, QByteArray> roleNames() const override;
};

#endif // SEARCHMODEL_H
