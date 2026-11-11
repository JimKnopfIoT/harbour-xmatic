#ifndef PUSHBANNERS_H
#define PUSHBANNERS_H

#include <QHash>
#include <QString>

// One banner per room, from the woken process or the app, recorded in the data
// directory so either can replace or close it. Rooms appear only as salted marks.

/// A room as the record names it.
QString pushRoomMark(const QString &salt, const QString &roomId);

/// Raises the banner of `roomId`, replacing that room's last one; without a room,
/// `slot` names it. `timestamp` is the announced message's, 0 where unknown.
void publishPushBanner(const QString &dataDirectory, const QString &roomId,
                       const QString &slot, const QString &summary, const QString &body,
                       bool audible, qint64 timestamp, int count = 0);

/// What the standing banners announced.
struct PushBannerRecord {
    QString salt;
    /// Room mark to the timestamp of the message its banner announced.
    QHash<QString, qint64> announced;
    /// Action key to room mark.
    QHash<QString, QString> keys;
};

PushBannerRecord readPushBanners(const QString &dataDirectory);

/// Closes the banner of `roomId`; false where none stood.
bool closePushBanner(const QString &dataDirectory, const QString &roomId);

/// Closes the banners that name no room: notices and the generic one.
void closePushNotices(const QString &dataDirectory);

/// Closes every recorded banner and empties the record.
void closePushBanners(const QString &dataDirectory);

/// The distributor dropped the registration.
void publishPushNotice(const QString &dataDirectory);

/// The app could not take the store from a push wake-up in time and did not start.
void publishStoreBusyNotice(const QString &dataDirectory);

/// Push from 0.45 or before is on but not carried over yet: the app has to run once.
void publishPushMigrationNotice(const QString &dataDirectory);

#endif // PUSHBANNERS_H
