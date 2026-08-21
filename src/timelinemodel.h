#ifndef TIMELINEMODEL_H
#define TIMELINEMODEL_H

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
    };

    explicit TimelineModel(QObject *parent = nullptr);

    QHash<int, QByteArray> roleNames() const override;

    /// Row of the event with this id, or -1. Lets the view jump to a pinned
    /// message that is already loaded.
    Q_INVOKABLE int indexOfEvent(const QString &eventId) const;
};

#endif // TIMELINEMODEL_H
