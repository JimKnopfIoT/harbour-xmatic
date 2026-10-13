#ifndef PUSHBANNERS_H
#define PUSHBANNERS_H

#include <QHash>
#include <QString>

// Banners the woken process raised, recorded in the data directory so the app
// can close them and open their rooms. A room appears there only as a salted mark.

/// A room as the record names it.
QString pushRoomMark(const QString &salt, const QString &roomId);

/// Raises the banner of `roomId`, replacing the last one; without a room, `slot`
/// names it. `timestamp` is the announced message's, 0 where unknown.
void publishPushBanner(const QString &dataDirectory, const QString &roomId,
                       const QString &slot, const QString &summary, const QString &body,
                       bool audible, qint64 timestamp);

/// What the closed banners had announced.
struct ClosedPushBanners {
    QString salt;
    /// Room mark to the timestamp of the message its banner announced.
    QHash<QString, qint64> announced;
    /// Action key to room mark.
    QHash<QString, QString> keys;
};

/// Closes every recorded banner and empties the record.
ClosedPushBanners closePushBanners(const QString &dataDirectory);

/// The distributor dropped the registration.
void publishPushNotice(const QString &dataDirectory);

/// The app could not take the store from a push wake-up in time and did not start.
void publishStoreBusyNotice(const QString &dataDirectory);

/// Push from 0.45 or before is on but not carried over yet: the app has to run once.
void publishPushMigrationNotice(const QString &dataDirectory);

#endif // PUSHBANNERS_H
