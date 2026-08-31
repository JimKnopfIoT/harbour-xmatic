#ifndef SEARCHMODEL_H
#define SEARCHMODEL_H

#include "difflistmodel.h"

/// Search hits inside one room.
///
/// Unlike the room list and the timeline, nothing streams here: the core
/// answers one page per command and the bridge turns that answer into a reset
/// (a fresh search) or an append (the next page). The diff machinery is
/// borrowed for the row storage alone.
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
