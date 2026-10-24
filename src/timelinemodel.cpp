#include "timelinemodel.h"

#include <QDateTime>
#include <QJsonArray>
#include <QSet>

namespace {

bool isBubbleKind(const QString &kind)
{
    return kind == QLatin1String("message") || kind == QLatin1String("undecryptable")
           || kind == QLatin1String("redacted");
}

/// The local day, or null without a timestamp - which then never matches.
QDate localDay(const QJsonObject &row)
{
    const double stamp = row.value(QStringLiteral("timestamp")).toDouble();
    return stamp > 0 ? QDateTime::fromMSecsSinceEpoch(qint64(stamp)).date() : QDate();
}

} // namespace

TimelineModel::TimelineModel(QObject *parent)
    : DiffListModel(parent)
{
    // Whenever the rows move, the read mark may move with them: recomputed in one
    // pass, queued. A reset is a different timeline and says nothing about this one.
    connect(this, &QAbstractItemModel::modelReset, this, [this] {
        m_readCounts.clear();
        scheduleReadCounts();
    });
    connect(this, &QAbstractItemModel::rowsInserted, this, [this] { scheduleReadCounts(); });
    connect(this, &QAbstractItemModel::rowsRemoved, this, [this] { scheduleReadCounts(); });
    connect(this, &QAbstractItemModel::dataChanged, this, [this] { scheduleReadCounts(); });

    connect(this, &QAbstractItemModel::modelReset, this, [this] { scheduleContinues(); });
    connect(this, &QAbstractItemModel::rowsInserted, this, [this] { scheduleContinues(); });
    connect(this, &QAbstractItemModel::rowsRemoved, this, [this] { scheduleContinues(); });
    connect(this, &QAbstractItemModel::dataChanged, this,
            [this](const QModelIndex &, const QModelIndex &, const QVector<int> &roles) {
                // Our own emit and the read marks come back here; neither moves a run.
                if (roles.isEmpty() || roles.contains(KindRole) || roles.contains(SenderRole)
                    || roles.contains(TimestampRole) || roles.contains(SystemRole)
                    || roles.contains(EventIdRole)) {
                    scheduleContinues();
                }
            });
}

QVariant TimelineModel::data(const QModelIndex &index, int role) const
{
    if (role == ContinuesFromRole) {
        return index.isValid() && index.row() < rows().count() ? continuesFrom(index.row())
                                                                : QString();
    }
    if (role == ReadMarkRole || role == ReadMarkByRole) {
        // Looked up by the row's own event id: by row number it was one off after
        // every page of older messages, and the mark stood under the wrong message.
        const QString eventId = DiffListModel::data(index, EventIdRole).toString();
        const int count = eventId.isEmpty() ? 0 : m_readCounts.value(eventId);
        return role == ReadMarkRole ? QVariant(count > 0) : QVariant(count);
    }
    if (role == ThreadCountRole && !m_threadRoots.isEmpty()) {
        // The row's own number first - the SDK's summary, kept live. The server's list
        // is the fallback for roots that summary does not know.
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

QString TimelineModel::continuesFrom(int row) const
{
    const QJsonObject &own = rows().at(row);
    if (!isBubbleKind(own.value(QStringLiteral("kind")).toString())) {
        return QString();
    }
    const QString sender = own.value(QStringLiteral("sender")).toString();
    const QDate day = localDay(own);
    if (sender.isEmpty() || !day.isValid()) {
        return QString();
    }
    // Back to the previous row that draws something: marker, start and hidden lines do not.
    for (int i = row - 1; i >= 0; --i) {
        const QJsonObject &prev = rows().at(i);
        const QString kind = prev.value(QStringLiteral("kind")).toString();
        if (isBubbleKind(kind)) {
            if (prev.value(QStringLiteral("sender")).toString() != sender
                || localDay(prev) != day) {
                return QString();
            }
            const QString eventId = prev.value(QStringLiteral("eventId")).toString();
            return eventId.isEmpty() ? prev.value(QStringLiteral("id")).toString() : eventId;
        }
        if (kind == QLatin1String("date")
            || (kind == QLatin1String("system")
                && !m_hiddenSystems.contains(prev.value(QStringLiteral("system")).toString()))) {
            return QString();
        }
    }
    return QString();
}

void TimelineModel::setHiddenSystems(const QStringList &tokens)
{
    const QSet<QString> hidden = tokens.toSet();
    if (hidden == m_hiddenSystems) {
        return;
    }
    m_hiddenSystems = hidden;
    emit hiddenSystemsChanged();
    scheduleContinues();
}

void TimelineModel::scheduleContinues()
{
    if (m_continuesQueued) {
        return;
    }
    m_continuesQueued = true;
    QMetaObject::invokeMethod(this, "refreshContinues", Qt::QueuedConnection);
}

void TimelineModel::refreshContinues()
{
    m_continuesQueued = false;
    if (!rows().isEmpty()) {
        emit dataChanged(index(0, 0), index(rows().count() - 1, 0),
                         QVector<int> { ContinuesFromRole });
    }
}

void TimelineModel::scheduleReadCounts()
{
    // The emit at the end of the recount comes back here through dataChanged.
    if (m_updatingReadCounts || m_readCountsQueued) {
        return;
    }
    m_readCountsQueued = true;
    QMetaObject::invokeMethod(this, "updateReadCounts", Qt::QueuedConnection);
}

void TimelineModel::updateReadCounts()
{
    m_readCountsQueued = false;

    // One pass from the newest row backwards, carrying everyone met: a receipt
    // marks the newest event read, so whoever appears at a row has read it.
    QHash<QString, int> counts;
    QSet<QString> seen;
    for (int i = rows().count() - 1; i >= 0; --i) {
        const QJsonObject &row = rows().at(i);
        const QJsonArray users = row.value(QStringLiteral("readByUsers")).toArray();
        for (const QJsonValue &user : users) {
            seen.insert(user.toString());
        }
        const QString eventId = row.value(QStringLiteral("eventId")).toString();
        if (!eventId.isEmpty()
            && row.value(QStringLiteral("own")).toBool()
            && row.value(QStringLiteral("kind")).toString() == QLatin1String("message")) {
            // Never below what a message already showed: a receipt is taken off the old
            // row before it lands on the new one, and the eye vanished for that moment.
            counts.insert(eventId, qMax(seen.count(), m_readCounts.value(eventId)));
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
    names.insert(PollRole, "poll");
    names.insert(LocationRole, "location");
    names.insert(SenderRole, "sender");
    names.insert(SenderNameRole, "senderName");
    names.insert(SenderAvatarRole, "senderAvatar");
    names.insert(SystemRole, "system");
    names.insert(NameRole, "name");
    names.insert(ReasonRole, "reason");
    names.insert(PreviousNameRole, "previousName");
    names.insert(OwnRole, "own");
    names.insert(TimestampRole, "timestamp");
    names.insert(EditedRole, "edited");
    names.insert(PendingRole, "pending");
    names.insert(SendStateRole, "sendState");
    names.insert(EditStateRole, "editState");
    names.insert(RedactionStateRole, "redactionState");
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
    names.insert(ContinuesFromRole, "continuesFrom");
    return names;
}

void TimelineModel::setPoll(const QString &eventId, const QJsonValue &poll)
{
    patchField(indexOfEvent(eventId), QStringLiteral("poll"), poll);
}

double TimelineModel::oldestTimestamp() const
{
    double oldest = 0;
    for (const QJsonObject &row : rows()) {
        const double stamp = row.value(QStringLiteral("timestamp")).toDouble();
        if (stamp > 0 && (oldest == 0 || stamp < oldest)) {
            oldest = stamp;
        }
    }
    return oldest;
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
