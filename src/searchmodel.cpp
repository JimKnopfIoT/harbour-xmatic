#include "searchmodel.h"

SearchModel::SearchModel(QObject *parent)
    : DiffListModel(parent)
{
}

QHash<int, QByteArray> SearchModel::roleNames() const
{
    QHash<int, QByteArray> names;
    names.insert(EventIdRole, "eventId");
    names.insert(SenderRole, "sender");
    names.insert(SenderNameRole, "senderName");
    names.insert(TimestampRole, "timestamp");
    names.insert(BodyRole, "body");
    return names;
}
