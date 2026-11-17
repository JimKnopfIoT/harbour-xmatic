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
    bool capped = false;
    for (const QJsonObject &row : rows()) {
        if (row.value(QStringLiteral("space")).toBool()
                || row.value(QStringLiteral("muted")).toBool()
                || row.value(QStringLiteral("membership")).toString()
                   != QLatin1String("joined")) {
            continue;
        }
        const int unread = counted(row, m_countMode);
        if (unread <= 0) {
            continue;
        }
        ++roomsWithNews;
        messages += unread;
        capped = capped || countedCapped(row, m_countMode);
    }
    if (roomsWithNews == m_unreadRooms && messages == m_unreadMessages
            && capped == m_unreadCapped) {
        return;
    }
    m_unreadRooms = roomsWithNews;
    m_unreadMessages = messages;
    m_unreadCapped = capped;
    emit unreadTotalsChanged();
}

int RoomListModel::counted(const QJsonObject &row, CountMode mode)
{
    if (mode == CountAll) {
        return row.value(QStringLiteral("unread")).toInt();
    }
    if (mode == CountNone) {
        return 0;
    }
    return qMax(row.value(QStringLiteral("notified")).toInt(),
                row.value(QStringLiteral("mentions")).toInt());
}

bool RoomListModel::countedCapped(const QJsonObject &row, CountMode mode)
{
    if (mode == CountNone) {
        return false;
    }
    return row.value(mode == CountAll ? QStringLiteral("unreadCapped")
                                      : QStringLiteral("notifiedCapped")).toBool();
}

void RoomListModel::setCountMode(CountMode mode)
{
    if (mode == m_countMode) {
        return;
    }
    m_countMode = mode;
    if (!rows().isEmpty()) {
        emit dataChanged(index(0), index(rows().count() - 1),
                         { UnreadRole, UnreadCappedRole, UncountedRole });
    }
    recountUnread();
}

bool RoomListModel::isInvite(const QString &roomId) const
{
    for (const QJsonObject &row : rows()) {
        if (row.value(QStringLiteral("id")).toString() == roomId) {
            return row.value(QStringLiteral("membership")).toString()
                    == QLatin1String("invited");
        }
    }
    return false;
}

QHash<int, QByteArray> RoomListModel::roleNames() const
{
    QHash<int, QByteArray> names;
    names.insert(IdRole, "id");
    names.insert(NameRole, "name");
    names.insert(UnreadRole, "unread");
    names.insert(UnreadCappedRole, "unreadCapped");
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
    // The last event of the room, as the core reduced it: a kind every list
    // names in its own words, and the text for the two kinds that have one.
    names.insert(PreviewKindRole, "previewKind");
    names.insert(PreviewTextRole, "previewText");
    // Marked unread by hand, here or elsewhere: unread with nothing counted.
    names.insert(MarkedUnreadRole, "markedUnread");
    // Unread, but nothing in it is counted: marked like a room marked by hand.
    names.insert(UncountedRole, "uncounted");
    return names;
}

void RoomListModel::setNotifyMode(const QString &roomId, const QString &mode)
{
    for (int i = 0; i < rows().count(); ++i) {
        QJsonObject row = rows().at(i);
        if (row.value(QStringLiteral("id")).toString() != roomId) {
            continue;
        }
        if (row.value(QStringLiteral("notifyMode")).toString() == mode) {
            return;
        }
        row.insert(QStringLiteral("notifyMode"), mode);
        row.insert(QStringLiteral("muted"), mode == QLatin1String("mute"));

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

void RoomListModel::setFields(const QString &roomId, const QJsonObject &fields)
{
    for (int i = 0; i < rows().count(); ++i) {
        QJsonObject row = rows().at(i);
        if (row.value(QStringLiteral("id")).toString() != roomId) {
            continue;
        }
        bool changed = false;
        for (auto it = fields.constBegin(); it != fields.constEnd(); ++it) {
            if (row.value(it.key()) != it.value()) {
                row.insert(it.key(), it.value());
                changed = true;
            }
        }
        if (!changed) {
            return;
        }
        QJsonObject operation;
        operation.insert(QStringLiteral("op"), QStringLiteral("set"));
        operation.insert(QStringLiteral("index"), i);
        operation.insert(QStringLiteral("value"), row);
        applyOperations(QJsonArray { operation });
        return;
    }
}

void RoomListModel::clearUnread(const QString &roomId)
{
    for (int i = 0; i < rows().count(); ++i) {
        QJsonObject row = rows().at(i);
        if (row.value(QStringLiteral("id")).toString() != roomId) {
            continue;
        }
        if (row.value(QStringLiteral("unread")).toInt() == 0
            && row.value(QStringLiteral("notifications")).toInt() == 0
            && row.value(QStringLiteral("notified")).toInt() == 0
            && row.value(QStringLiteral("mentions")).toInt() == 0
            && !row.value(QStringLiteral("markedUnread")).toBool()) {
            return;
        }
        row.insert(QStringLiteral("unread"), 0);
        row.insert(QStringLiteral("notifications"), 0);
        row.insert(QStringLiteral("notified"), 0);
        row.insert(QStringLiteral("mentions"), 0);
        row.insert(QStringLiteral("markedUnread"), false);

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
        // A room without a name is shown by its identifier rather than as an empty row;
        // that happens while the first sync is still filling state in.
        const QString name = row.value(QStringLiteral("name")).toString();
        return name.isEmpty() ? row.value(QStringLiteral("id")).toString() : name;
    }
    if (role == UnreadRole) {
        return counted(row, m_countMode);
    }
    if (role == UnreadCappedRole) {
        return countedCapped(row, m_countMode);
    }
    if (role == UncountedRole) {
        return row.value(QStringLiteral("unread")).toInt() > 0 && counted(row, m_countMode) == 0;
    }
    return DiffListModel::valueFor(row, role);
}
