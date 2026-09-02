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
        SenderRole,
        SenderNameRole,
        SenderAvatarRole,
        SystemRole,
        NameRole,
        OwnRole,
        TimestampRole,
        EditedRole,
        PendingRole,
        SendStateRole,
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
