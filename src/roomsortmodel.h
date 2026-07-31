#ifndef ROOMSORTMODEL_H
#define ROOMSORTMODEL_H

#include <QSortFilterProxyModel>

/// Groups the room list without touching the SDK's own ordering: favourites to
/// the top, low-priority rooms to the bottom, everything else in between. Within
/// each group the source order is kept verbatim, so the SDK's activity sort
/// still decides who is where — this only lifts and sinks the two tagged groups.
///
/// Sorting lives here rather than in the core because the core streams
/// `VectorDiff`s whose indices map one-to-one onto the source model; reordering
/// there would break that mapping. A proxy keeps the diff bookkeeping intact and
/// re-sorts on top of it.
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
