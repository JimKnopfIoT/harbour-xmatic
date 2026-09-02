#ifndef ROOMSORTMODEL_H
#define ROOMSORTMODEL_H

#include <QSortFilterProxyModel>

/// Groups the list without touching the SDK's order: favourites up, low priority
/// down, source order kept inside each. A proxy, so the diff indices stay valid.
class RoomSortModel : public QSortFilterProxyModel
{
    Q_OBJECT

public:
    explicit RoomSortModel(QObject *parent = nullptr);

protected:
    bool lessThan(const QModelIndex &left, const QModelIndex &right) const override;

private:
    int groupOf(const QModelIndex &sourceIndex) const;
};

#endif // ROOMSORTMODEL_H
