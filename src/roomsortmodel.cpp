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
    // Same group: keep the source (SDK) order. The proxy sort is not guaranteed
    // stable, so the tie is broken explicitly on the source row.
    return left.row() < right.row();
}
