#include "difflistmodel.h"

#include <QJsonValue>

DiffListModel::DiffListModel(QObject *parent)
    : QAbstractListModel(parent)
{
}

int DiffListModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_rows.count();
}

QVariant DiffListModel::data(const QModelIndex &index, int role) const
{
    if (index.row() < 0 || index.row() >= m_rows.count()) {
        return QVariant();
    }
    return valueFor(m_rows.at(index.row()), role);
}

const QByteArray &DiffListModel::fieldFor(int role) const
{
    if (m_fields.isEmpty()) {
        m_fields = roleNames();
    }
    static const QByteArray none;
    const auto found = m_fields.constFind(role);
    return found == m_fields.constEnd() ? none : *found;
}

QVariant DiffListModel::valueFor(const QJsonObject &row, int role) const
{
    const QByteArray &field = fieldFor(role);
    if (field.isEmpty()) {
        return QVariant();
    }
    return row.value(QString::fromLatin1(field)).toVariant();
}

void DiffListModel::patchField(int row, const QString &field, const QJsonValue &value)
{
    if (row < 0 || row >= m_rows.count()) {
        return;
    }
    if (m_rows.at(row).value(field) == value) {
        return;
    }
    m_rows[row].insert(field, value);
    const QModelIndex changed = createIndex(row, 0);
    emit dataChanged(changed, changed);
}

/// An operation whose index the model cannot satisfy: dropped, because applying
/// it would be worse - but never silently, every later index is then off.
void DiffListModel::reportDrift(const QString &op, int index)
{
    qWarning("xmatic: diff out of step (%s at %d, %d rows) - the model and the core disagree",
             qPrintable(op), index, m_rows.count());
    if (!m_outOfStep) {
        m_outOfStep = true;
        emit outOfStep();
    }
}

void DiffListModel::applyOperations(const QJsonArray &operations)
{
    const int before = m_rows.count();

    for (const QJsonValue &value : operations) {
        applyOperation(value.toObject());
    }

    if (m_rows.count() != before) {
        emit countChanged();
    }
}

void DiffListModel::applyOperation(const QJsonObject &operation)
{
    const QString op = operation.value(QStringLiteral("op")).toString();
    const int index = operation.value(QStringLiteral("index")).toInt();

    if (op == QLatin1String("append")) {
        const QJsonArray values = operation.value(QStringLiteral("values")).toArray();
        if (values.isEmpty()) {
            return;
        }
        beginInsertRows(QModelIndex(), m_rows.count(), m_rows.count() + values.count() - 1);
        for (const QJsonValue &value : values) {
            m_rows.append(value.toObject());
        }
        endInsertRows();
        return;
    }

    if (op == QLatin1String("insert")) {
        if (index < 0 || index > m_rows.count()) {
            reportDrift(op, index);
            return;
        }
        beginInsertRows(QModelIndex(), index, index);
        m_rows.insert(index, operation.value(QStringLiteral("value")).toObject());
        endInsertRows();
        return;
    }

    if (op == QLatin1String("set")) {
        if (index < 0 || index >= m_rows.count()) {
            reportDrift(op, index);
            return;
        }
        const QJsonObject value = operation.value(QStringLiteral("value")).toObject();
        // `dataChanged` without roles re-evaluates every binding of the row, and the
        // SDK sets rows identical to what is stored - a moving receipt does it often.
        if (m_rows.at(index) == value) {
            return;
        }
        m_rows[index] = value;
        const QModelIndex changed = createIndex(index, 0);
        emit dataChanged(changed, changed);
        return;
    }

    if (op == QLatin1String("remove")) {
        if (index < 0 || index >= m_rows.count()) {
            reportDrift(op, index);
            return;
        }
        beginRemoveRows(QModelIndex(), index, index);
        m_rows.remove(index);
        endRemoveRows();
        return;
    }

    if (op == QLatin1String("popBack")) {
        if (m_rows.isEmpty()) {
            reportDrift(op, -1);
            return;
        }
        const int last = m_rows.count() - 1;
        beginRemoveRows(QModelIndex(), last, last);
        m_rows.remove(last);
        endRemoveRows();
        return;
    }

    if (op == QLatin1String("truncate")) {
        const int length = operation.value(QStringLiteral("length")).toInt();
        // Equal is a truncate to what is there already; past it is a model behind.
        if (length < 0 || length > m_rows.count()) {
            reportDrift(op, length);
            return;
        }
        if (length == m_rows.count()) {
            return;
        }
        beginRemoveRows(QModelIndex(), length, m_rows.count() - 1);
        m_rows.remove(length, m_rows.count() - length);
        endRemoveRows();
        return;
    }

    if (op == QLatin1String("clear")) {
        clear();
        return;
    }

    if (op == QLatin1String("reset")) {
        const QJsonArray values = operation.value(QStringLiteral("values")).toArray();
        beginResetModel();
        m_rows.clear();
        for (const QJsonValue &value : values) {
            m_rows.append(value.toObject());
        }
        m_outOfStep = false;
        endResetModel();
        return;
    }

    reportDrift(op, index);
}

void DiffListModel::clear()
{
    m_outOfStep = false;
    if (m_rows.isEmpty()) {
        return;
    }
    beginResetModel();
    m_rows.clear();
    endResetModel();
}
