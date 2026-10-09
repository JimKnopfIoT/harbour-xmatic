#ifndef ROOMSORTMODEL_H
#define ROOMSORTMODEL_H

#include <QSortFilterProxyModel>

/// Groups the list without touching the SDK's order: favourites up, low priority
/// down; inside each the user's order, else the source's. A proxy, so the diff
/// indices stay valid.
class RoomSortModel : public QSortFilterProxyModel
{
    Q_OBJECT

public:
    explicit RoomSortModel(QObject *parent = nullptr);

    /// By name instead of by activity, and rooms with unread messages first.
    void setOrder(bool byName, bool unreadFirst);

protected:
    bool lessThan(const QModelIndex &left, const QModelIndex &right) const override;

private:
    int groupOf(const QModelIndex &sourceIndex) const;
    bool m_byName = false;
    bool m_unreadFirst = false;
};

#endif // ROOMSORTMODEL_H
