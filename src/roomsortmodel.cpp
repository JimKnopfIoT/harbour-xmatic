#include "roomsortmodel.h"

#include "roomlistmodel.h"

RoomSortModel::RoomSortModel(QObject *parent)
    : QSortFilterProxyModel(parent)
{
    // Re-sort as rows change (tags flip, the SDK moves a room on new activity)
    // instead of only once at load.
    setDynamicSortFilter(true);
    sort(0);
}

void RoomSortModel::setOrder(bool byName, bool unreadFirst)
{
    if (byName == m_byName && unreadFirst == m_unreadFirst) {
        return;
    }
    m_byName = byName;
    m_unreadFirst = unreadFirst;
    invalidate();
}

int RoomSortModel::groupOf(const QModelIndex &sourceIndex) const
{
    // The two are mutually exclusive in the core, so the order does not matter -
    // but favourite wins if that ever changes.
    if (sourceIndex.data(RoomListModel::FavouriteRole).toBool()) {
        return 0;
    }
    if (sourceIndex.data(RoomListModel::LowPriorityRole).toBool()) {
        return 2;
    }
    return 1;
}

bool RoomSortModel::lessThan(const QModelIndex &left, const QModelIndex &right) const
{
    const int leftGroup = groupOf(left);
    const int rightGroup = groupOf(right);
    if (leftGroup != rightGroup) {
        return leftGroup < rightGroup;
    }
    if (m_unreadFirst) {
        const bool leftUnread = left.data(RoomListModel::UnreadRole).toInt() > 0;
        const bool rightUnread = right.data(RoomListModel::UnreadRole).toInt() > 0;
        if (leftUnread != rightUnread) {
            return leftUnread;
        }
    }
    if (m_byName) {
        const int byName = QString::localeAwareCompare(
                left.data(RoomListModel::NameRole).toString(),
                right.data(RoomListModel::NameRole).toString());
        if (byName != 0) {
            return byName < 0;
        }
    }
    // Otherwise keep the source (SDK) order. The proxy sort is not guaranteed
    // stable, so the tie is broken explicitly on the source row.
    return left.row() < right.row();
}
