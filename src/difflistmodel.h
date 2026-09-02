#ifndef DIFFLISTMODEL_H
#define DIFFLISTMODEL_H

#include <QAbstractListModel>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QVector>

/// A list model fed by the core's `VectorDiff` stream; a subclass supplies only
/// `roleNames()`. Never reordered or filtered - the core owns the order.
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
    /// Says that a diff could not be applied. See the definition.
    void reportDrift(const QString &op, int index) const;
    const QByteArray &fieldFor(int role) const;

    QVector<QJsonObject> m_rows;
    /// roleNames() builds its table anew on every call, and data() is asked
    /// once per role per row per repaint. Built once here instead.
    mutable QHash<int, QByteArray> m_fields;
};

#endif // DIFFLISTMODEL_H
