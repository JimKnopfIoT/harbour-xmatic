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

    /// The room's thread roots as the server lists them, event id to reply
    /// count.
    ///
    /// The marker that opens a thread hangs on the SDK's thread summary, and a
    /// root that was cached before threading was switched on carries none - on
    /// one test device every thread was reachable and on the other none was,
    /// same version, same room, different store. The server's `/threads`
    /// answer is laid over the rows so the way in does not depend on what the
    /// cache happens to hold.
    void setThreadRoots(const QHash<QString, int> &roots);

private:
    /// How many other people have read each own message.
    ///
    /// A read receipt marks the newest event someone has read, and that is
    /// usually not one of our own messages - in a running conversation it sits
    /// on the other side's own latest message. Reading the receipt off the row
    /// it hangs on therefore showed nothing under our messages, which is what
    /// "I don't see that you read my messages" was. Counted instead in one
    /// pass from the newest row backwards, so every own message says how many
    /// people got at least that far.
    void updateReadCounts();

    QVector<int> m_readCounts;
    bool m_updatingReadCounts = false;
    QHash<QString, int> m_threadRoots;
};

#endif // TIMELINEMODEL_H
