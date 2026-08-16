#include "roomlistmodel.h"

#include <QJsonArray>

RoomListModel::RoomListModel(QObject *parent)
    : DiffListModel(parent)
{
    // Every way a row can change goes through these four signals, so the
    // totals can only be recomputed here — no caller has to remember to.
    connect(this, &QAbstractItemModel::rowsInserted,
            this, &RoomListModel::recountUnread);
    connect(this, &QAbstractItemModel::rowsRemoved,
            this, &RoomListModel::recountUnread);
    connect(this, &QAbstractItemModel::modelReset,
            this, &RoomListModel::recountUnread);
    connect(this, &QAbstractItemModel::dataChanged,
            this, &RoomListModel::recountUnread);
}

void RoomListModel::recountUnread()
{
    int roomsWithNews = 0;
    int messages = 0;
    for (const QJsonObject &row : rows()) {
        if (row.value(QStringLiteral("space")).toBool()
                || row.value(QStringLiteral("muted")).toBool()
                || row.value(QStringLiteral("membership")).toString()
                   != QLatin1String("joined")) {
            continue;
        }
        const int unread = row.value(QStringLiteral("unread")).toInt();
        if (unread <= 0) {
            continue;
        }
        ++roomsWithNews;
        messages += unread;
    }
    if (roomsWithNews == m_unreadRooms && messages == m_unreadMessages) {
        return;
    }
    m_unreadRooms = roomsWithNews;
    m_unreadMessages = messages;
    emit unreadTotalsChanged();
}

QHash<int, QByteArray> RoomListModel::roleNames() const
{
    QHash<int, QByteArray> names;
    names.insert(IdRole, "id");
    names.insert(NameRole, "name");
    names.insert(UnreadRole, "unread");
    names.insert(MentionsRole, "mentions");
    names.insert(EncryptedRole, "encrypted");
    names.insert(SpaceRole, "space");
    names.insert(MembershipRole, "membership");
    names.insert(AvatarRole, "avatar");
    names.insert(TimestampRole, "timestamp");
    names.insert(MutedRole, "muted");
    names.insert(FavouriteRole, "favourite");
    names.insert(LowPriorityRole, "lowPriority");
    names.insert(TombstonedRole, "tombstoned");
    return names;
}

void RoomListModel::setMuted(const QString &roomId, bool muted)
{
    for (int i = 0; i < rows().count(); ++i) {
        QJsonObject row = rows().at(i);
        if (row.value(QStringLiteral("id")).toString() != roomId) {
            continue;
        }
        if (row.value(QStringLiteral("muted")).toBool() == muted) {
            return;
        }
        row.insert(QStringLiteral("muted"), muted);

        // Goes through the same path a diff from the core takes, so the row
        // stays a plain replacement and the indices cannot drift.
        QJsonObject operation;
        operation.insert(QStringLiteral("op"), QStringLiteral("set"));
        operation.insert(QStringLiteral("index"), i);
        operation.insert(QStringLiteral("value"), row);
        applyOperations(QJsonArray { operation });
        return;
    }
}

QVariant RoomListModel::valueFor(const QJsonObject &row, int role) const
{
    if (role == NameRole) {
        // A room without a name is shown by its identifier rather than as an
        // empty row; that happens while the first sync is still filling in
        // state.
        const QString name = row.value(QStringLiteral("name")).toString();
        return name.isEmpty() ? row.value(QStringLiteral("id")).toString() : name;
    }
    return DiffListModel::valueFor(row, role);
}
