#include "timelinemodel.h"

#include <QJsonArray>
#include <QSet>

TimelineModel::TimelineModel(QObject *parent)
    : DiffListModel(parent)
{
    // Whenever the rows move, the read mark may move with them. Recomputed
    // here rather than in data(): it is one pass over the list, and asking per
    // row would make every redraw quadratic.
    connect(this, &QAbstractItemModel::modelReset, this, &TimelineModel::updateReadCounts);
    connect(this, &QAbstractItemModel::rowsInserted, this, &TimelineModel::updateReadCounts);
    connect(this, &QAbstractItemModel::rowsRemoved, this, &TimelineModel::updateReadCounts);
    connect(this, &QAbstractItemModel::dataChanged, this, &TimelineModel::updateReadCounts);
}

QVariant TimelineModel::data(const QModelIndex &index, int role) const
{
    if (role == ReadMarkRole) {
        return m_readCounts.value(index.row()) > 0;
    }
    if (role == ReadMarkByRole) {
        return m_readCounts.value(index.row());
    }
    if (role == ThreadCountRole && !m_threadRoots.isEmpty()) {
        // The row's own number first: it comes from the SDK's summary and is
        // the live one, kept up to date as replies arrive. The server's list is
        // the fallback for the roots that summary does not know - it is fetched
        // once when the room opens and does not grow afterwards.
        const int own = DiffListModel::data(index, role).toInt();
        if (own > 0) {
            return own;
        }
        const QString id = DiffListModel::data(index, EventIdRole).toString();
        if (!id.isEmpty()) {
            const int listed = m_threadRoots.value(id, 0);
            if (listed > 0) {
                return listed;
            }
        }
    }
    return DiffListModel::data(index, role);
}

void TimelineModel::setThreadRoots(const QHash<QString, int> &roots)
{
    if (roots == m_threadRoots) {
        return;
    }
    m_threadRoots = roots;
    if (rowCount() > 0) {
        emit dataChanged(index(0), index(rowCount() - 1), QVector<int>() << ThreadCountRole);
    }
}

void TimelineModel::updateReadCounts()
{
    // The emits below come back here through dataChanged.
    if (m_updatingReadCounts) {
        return;
    }

    // One pass from the newest row backwards, carrying everyone met so far.
    // A receipt marks the newest event a person has read, so whoever appears
    // at row i has read row i and everything before it - the running set is
    // therefore exactly "who has read this far" for each row in turn. Their
    // own messages count as read by them, which is what the SDK's implicit
    // receipt on a sent event says.
    QVector<int> counts(rows().count(), 0);
    QSet<QString> seen;
    for (int i = rows().count() - 1; i >= 0; --i) {
        const QJsonObject &row = rows().at(i);
        const QJsonArray users = row.value(QStringLiteral("readByUsers")).toArray();
        for (const QJsonValue &user : users) {
            seen.insert(user.toString());
        }
        if (row.value(QStringLiteral("own")).toBool()
            && row.value(QStringLiteral("kind")).toString() == QLatin1String("message")) {
            counts[i] = seen.count();
        }
    }

    if (counts == m_readCounts) {
        return;
    }
    m_readCounts = counts;

    if (rows().isEmpty()) {
        return;
    }
    m_updatingReadCounts = true;
    emit dataChanged(index(0, 0), index(rows().count() - 1, 0),
                     QVector<int> { ReadMarkRole, ReadMarkByRole });
    m_updatingReadCounts = false;
}

QHash<int, QByteArray> TimelineModel::roleNames() const
{
    QHash<int, QByteArray> names;
    names.insert(IdRole, "id");
    names.insert(EventIdRole, "eventId");
    names.insert(EditableRole, "editable");
    names.insert(MediaRole, "media");
    names.insert(ReplyToRole, "replyTo");
    names.insert(KindRole, "kind");
    names.insert(BodyRole, "body");
    names.insert(FormattedRole, "formatted");
    names.insert(MsgTypeRole, "msgtype");
    names.insert(SenderRole, "sender");
    names.insert(SenderNameRole, "senderName");
    names.insert(SenderAvatarRole, "senderAvatar");
    names.insert(SystemRole, "system");
    names.insert(NameRole, "name");
    names.insert(OwnRole, "own");
    names.insert(TimestampRole, "timestamp");
    names.insert(EditedRole, "edited");
    names.insert(PendingRole, "pending");
    names.insert(SendStateRole, "sendState");
    names.insert(ThreadRootRole, "threadRoot");
    names.insert(ThreadCountRole, "threadCount");
    names.insert(UtdCauseRole, "utdCause");
    names.insert(ReadByRole, "readBy");
    names.insert(CaptionRole, "caption");
    names.insert(TxnIdRole, "txnId");
    names.insert(ReactionsRole, "reactions");
    names.insert(ReadMarkRole, "readMark");
    names.insert(ReadMarkByRole, "readMarkBy");
    names.insert(ShieldRole, "shield");
    return names;
}

int TimelineModel::indexOfEvent(const QString &eventId) const
{
    if (eventId.isEmpty()) {
        return -1;
    }
    for (int i = 0; i < rows().count(); ++i) {
        if (rows().at(i).value(QStringLiteral("eventId")).toString() == eventId) {
            return i;
        }
    }
    return -1;
}
