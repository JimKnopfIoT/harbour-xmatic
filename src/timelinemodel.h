#ifndef TIMELINEMODEL_H
#define TIMELINEMODEL_H

#include <QHash>
#include <QString>
#include <QVector>

#include "difflistmodel.h"

/// A room's timeline. Edits, local echoes and decryption all happen in the
/// core; rows arrive here already resolved.
class TimelineModel : public DiffListModel
{
    Q_OBJECT

public:
    enum Role {
        IdRole = Qt::UserRole + 1,
        EventIdRole,
        EditableRole,
        MediaRole,
        ReplyToRole,
        KindRole,
        BodyRole,
        FormattedRole,
        MsgTypeRole,
        /// The poll's question, answers and - unless it is undisclosed and still
        /// running - the counts. Null for every other row.
        PollRole,
        /// Point, and for a live share its window. Null for every other row.
        LocationRole,
        SenderRole,
        SenderNameRole,
        SenderAvatarRole,
        SystemRole,
        NameRole,
        /// The reason a moderator gave for a removal or a ban. Null elsewhere.
        ReasonRole,
        /// The display name before a profile change. Null for other rows.
        PreviousNameRole,
        OwnRole,
        TimestampRole,
        EditedRole,
        PendingRole,
        SendStateRole,
        /// Our own edit or deletion of the row, while the queue holds it:
        /// "", "sending" or "failed".
        EditStateRole,
        RedactionStateRole,
        ThreadRootRole,
        ThreadCountRole,
        UtdCauseRole,
        ReadByRole,
        CaptionRole,
        TxnIdRole,
        ReactionsRole,
        /// Whether anybody has read this own message, and how many. Derived
        /// from the whole list, not from the row - see updateReadCounts().
        ReadMarkRole,
        ReadMarkByRole,
        ShieldRole,
    };

    explicit TimelineModel(QObject *parent = nullptr);

    QHash<int, QByteArray> roleNames() const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;

    /// Row of the event with this id, or -1. Lets the view jump to a pinned
    /// message that is already loaded.
    Q_INVOKABLE int indexOfEvent(const QString &eventId) const;

    /// Oldest loaded timestamp, 0 if none.
    Q_INVOKABLE double oldestTimestamp() const;

    /// The poll as the core vouches for it after checking who ended it.
    void setPoll(const QString &eventId, const QJsonValue &poll);

    /// The server's thread roots, event id to reply count. A root cached before
    /// threading has no SDK summary, so the way in would depend on the store.
    void setThreadRoots(const QHash<QString, int> &roots);

private slots:
    /// How many others have read each own message. A receipt sits on the other
    /// side's own latest message, so reading it off its row showed nothing.
    void updateReadCounts();

private:
    /// Asks for a recount after the current signal: a `dataChanged` emitted while
    /// the view works through another is dropped, and the mark stood one row off.
    void scheduleReadCounts();

    /// By event id, not row number: a page of older messages shifts every row. The
    /// value never falls - for a moment between rows nobody has read anything.
    QHash<QString, int> m_readCounts;
    bool m_updatingReadCounts = false;
    bool m_readCountsQueued = false;
    QHash<QString, int> m_threadRoots;
};

#endif // TIMELINEMODEL_H
