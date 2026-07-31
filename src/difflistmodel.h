#ifndef DIFFLISTMODEL_H
#define DIFFLISTMODEL_H

#include <QAbstractListModel>
#include <QJsonArray>
#include <QJsonObject>
#include <QVector>

/// A list model fed by the diff stream the Rust core produces.
///
/// The SDK expresses both the room list and a timeline as `VectorDiff`s, whose
/// vocabulary maps one to one onto the model's insert/remove/change signals.
/// Everything a subclass has to supply is `roleNames()`; by default each role
/// reads the JSON member of the same name, which keeps core and UI naming in
/// step.
///
/// Rows are never reordered or filtered here — the core owns the order, and
/// diverging from it would put the model's indices out of step with the diffs.
class DiffListModel : public QAbstractListModel
{
    Q_OBJECT

    Q_PROPERTY(int count READ count NOTIFY countChanged)

public:
    explicit DiffListModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;

    int count() const { return m_rows.count(); }

    /// Applies one batch of operations from the core.
    void applyOperations(const QJsonArray &operations);

    /// Drops every row, for instance when the session ends.
    void clear();

signals:
    void countChanged();

protected:
    /// Reads one role from a row. The default takes the JSON member named like
    /// the role; override to add fallbacks or derived values.
    virtual QVariant valueFor(const QJsonObject &row, int role) const;

    /// The rows, readable by subclasses for lookups such as finding an event
    /// by id. Only `applyOperation` mutates them.
    const QVector<QJsonObject> &rows() const { return m_rows; }

private:
    void applyOperation(const QJsonObject &operation);

    QVector<QJsonObject> m_rows;
};

#endif // DIFFLISTMODEL_H
