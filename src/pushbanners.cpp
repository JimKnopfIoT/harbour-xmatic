#include "pushbanners.h"

#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDataStream>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QStringList>
#include <QUuid>
#include <QVariant>
#include <QVariantMap>

namespace {

/// A blocking call to lipstick must not hold up a yield.
const int NotifyTimeoutMs = 2000;

QString recordPath(const QString &dataDirectory)
{
    return dataDirectory + QStringLiteral("/push-banners.json");
}

QString freshSalt()
{
    return QString::fromLatin1(QUuid::createUuid().toRfc4122().toHex());
}

struct Record {
    QString salt;
    /// Room mark, or a slot name for banners without a room, to `{id, key, ts}`.
    QJsonObject banners;
    /// Minted on this read: the salt is not on disk yet.
    bool fresh = false;
};

Record readRecord(const QString &dataDirectory)
{
    Record record;
    QFile file(recordPath(dataDirectory));
    const QJsonObject root = file.open(QIODevice::ReadOnly)
            ? QJsonDocument::fromJson(file.readAll()).object()
            : QJsonObject();
    if (root.value(QStringLiteral("banners")).isObject()) {
        record.salt = root.value(QStringLiteral("salt")).toString();
        record.banners = root.value(QStringLiteral("banners")).toObject();
    }
    if (record.salt.isEmpty()) {
        record.salt = freshSalt();
        record.fresh = true;
        // The record of 0.46 named rooms in clear: marks from here on.
        if (!root.contains(QStringLiteral("banners"))) {
            for (auto it = root.constBegin(); it != root.constEnd(); ++it) {
                const QString slot = it.key().startsWith(QLatin1Char('!'))
                        ? pushRoomMark(record.salt, it.key())
                        : it.key();
                record.banners.insert(slot, it.value());
            }
        }
    }
    return record;
}

void writeRecord(const QString &dataDirectory, const Record &record)
{
    if (record.banners.isEmpty()) {
        QFile::remove(recordPath(dataDirectory));
        return;
    }
    QJsonObject root;
    root.insert(QStringLiteral("salt"), record.salt);
    root.insert(QStringLiteral("banners"), record.banners);
    QSaveFile file(recordPath(dataDirectory));
    if (file.open(QIODevice::WriteOnly)) {
        file.write(QJsonDocument(root).toJson(QJsonDocument::Compact));
        file.commit();
    }
}

/// A plain message: QDBusInterface introspects first, blocking without a bound.
QDBusMessage callNotifications(const QString &method, const QVariantList &arguments)
{
    QDBusMessage call = QDBusMessage::createMethodCall(
            QStringLiteral("org.freedesktop.Notifications"),
            QStringLiteral("/org/freedesktop/Notifications"),
            QStringLiteral("org.freedesktop.Notifications"), method);
    call.setArguments(arguments);
    return QDBusConnection::sessionBus().call(call, QDBus::Block, NotifyTimeoutMs);
}

} // namespace

QString pushRoomMark(const QString &salt, const QString &roomId)
{
    QCryptographicHash hash(QCryptographicHash::Sha256);
    hash.addData(salt.toUtf8());
    hash.addData(QByteArray(1, '\0'));
    hash.addData(roomId.toUtf8());
    return QString::fromLatin1(hash.result().toHex());
}

/// Without Qt Quick - the woken process has no QML engine. The category is what
/// makes it audible; a banner with a room opens it, one without opens the app.
void publishPushBanner(const QString &dataDirectory, const QString &roomId,
                       const QString &slot, const QString &summary, const QString &body,
                       bool audible, qint64 timestamp, int count)
{
    QVariantMap hints;
    // Tone, vibration and LED hang off the category. Left off where the push
    // rules call this one quiet - the preview hints still show the banner.
    if (audible) {
        hints.insert(QStringLiteral("category"), QStringLiteral("x-nemo.messaging.im"));
    }
    // Summary and body alone only fill the event feed. The banner that slides
    // in over whatever is on screen is this pair.
    hints.insert(QStringLiteral("x-nemo-preview-summary"), summary);
    hints.insert(QStringLiteral("x-nemo-preview-body"), body);
    hints.insert(QStringLiteral("x-nemo-owner"), QStringLiteral("xmatic"));
    if (count > 0) {
        hints.insert(QStringLiteral("x-nemo-item-count"), count);
    }

    Record record = readRecord(dataDirectory);
    const QString name = roomId.isEmpty() ? slot : pushRoomMark(record.salt, roomId);
    QJsonObject entry = record.banners.value(name).toObject();
    const uint replaces = uint(entry.value(QStringLiteral("id")).toDouble());

    // The action carries an opaque key, not the room id; the app maps it back.
    QString action = QStringLiteral("org.xmatic.xmatic /org/xmatic/xmatic org.xmatic.xmatic activate");
    if (!roomId.isEmpty()) {
        QString key = entry.value(QStringLiteral("key")).toString();
        if (key.isEmpty()) {
            key = QString::fromLatin1(QUuid::createUuid().toRfc4122().toHex());
            entry.insert(QStringLiteral("key"), key);
        }
        QByteArray argument;
        QDataStream stream(&argument, QIODevice::WriteOnly);
        stream << QVariant(key);
        action = QStringLiteral("org.xmatic.xmatic /org/xmatic/xmatic org.xmatic.xmatic openPushBanner ")
                + QString::fromLatin1(argument.toBase64());
    }
    hints.insert(QStringLiteral("x-nemo-remote-action-default"), action);
    hints.insert(QStringLiteral("x-nemo-remote-action-app"), action);

    const QStringList actions{QStringLiteral("default"), QString(),
                              QStringLiteral("app"), QString()};
    const QDBusMessage reply = callNotifications(QStringLiteral("Notify"), QVariantList{
            QStringLiteral("xmatic"),
            replaces,
            QStringLiteral("/usr/share/icons/hicolor/86x86/apps/harbour-xmatic.png"),
            summary,
            body,
            actions,
            hints,
            -1});
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty()) {
        qWarning("xmatic: the push banner was not raised");
        return;
    }
    entry.insert(QStringLiteral("id"), double(reply.arguments().first().toUInt()));
    entry.insert(QStringLiteral("ts"), double(qMax<qint64>(timestamp,
            qint64(entry.value(QStringLiteral("ts")).toDouble()))));
    record.banners.insert(name, entry);
    writeRecord(dataDirectory, record);
}

PushBannerRecord readPushBanners(const QString &dataDirectory)
{
    PushBannerRecord standing;
    if (!QFile::exists(recordPath(dataDirectory))) {
        return standing;
    }
    const Record record = readRecord(dataDirectory);
    // Marks made under a salt that is not kept would never match again.
    if (record.fresh) {
        writeRecord(dataDirectory, record);
    }
    standing.salt = record.salt;
    for (auto it = record.banners.constBegin(); it != record.banners.constEnd(); ++it) {
        const QJsonObject entry = it.value().toObject();
        const QString key = entry.value(QStringLiteral("key")).toString();
        if (key.isEmpty()) {
            continue;
        }
        standing.keys.insert(key, it.key());
        standing.announced.insert(it.key(), qint64(entry.value(QStringLiteral("ts")).toDouble()));
    }
    return standing;
}

bool closePushBanner(const QString &dataDirectory, const QString &roomId)
{
    if (roomId.isEmpty() || !QFile::exists(recordPath(dataDirectory))) {
        return false;
    }
    Record record = readRecord(dataDirectory);
    const QString name = pushRoomMark(record.salt, roomId);
    if (!record.banners.contains(name)) {
        return false;
    }
    callNotifications(QStringLiteral("CloseNotification"),
                      QVariantList{uint(record.banners.value(name).toObject()
                                                .value(QStringLiteral("id")).toDouble())});
    record.banners.remove(name);
    writeRecord(dataDirectory, record);
    return true;
}

void closePushNotices(const QString &dataDirectory)
{
    if (!QFile::exists(recordPath(dataDirectory))) {
        return;
    }
    Record record = readRecord(dataDirectory);
    const QStringList names = record.banners.keys();
    bool closed = false;
    for (const QString &name : names) {
        const QJsonObject entry = record.banners.value(name).toObject();
        if (!entry.value(QStringLiteral("key")).toString().isEmpty()) {
            continue;
        }
        callNotifications(QStringLiteral("CloseNotification"),
                          QVariantList{uint(entry.value(QStringLiteral("id")).toDouble())});
        record.banners.remove(name);
        closed = true;
    }
    if (closed) {
        writeRecord(dataDirectory, record);
    }
}

void closePushBanners(const QString &dataDirectory)
{
    if (!QFile::exists(recordPath(dataDirectory))) {
        return;
    }
    const Record record = readRecord(dataDirectory);
    for (auto it = record.banners.constBegin(); it != record.banners.constEnd(); ++it) {
        callNotifications(QStringLiteral("CloseNotification"),
                          QVariantList{uint(it.value().toObject()
                                                    .value(QStringLiteral("id")).toDouble())});
    }
    QFile::remove(recordPath(dataDirectory));
}

void publishPushNotice(const QString &dataDirectory)
{
    publishPushBanner(dataDirectory, QString(), QStringLiteral("notice"),
                      QStringLiteral("xmatic"),
                      QCoreApplication::translate(
                          "PushWake",
                          "The push distributor dropped xmatic. Turn push back on under Account."),
                      false, 0);
}

void publishStoreBusyNotice(const QString &dataDirectory)
{
    publishPushBanner(dataDirectory, QString(), QStringLiteral("notice"),
                      QStringLiteral("xmatic"),
                      QCoreApplication::translate(
                          "PushWake",
                          "xmatic is still handling a notification. Open it again in a moment."),
                      false, 0);
}

void publishPushMigrationNotice(const QString &dataDirectory)
{
    publishPushBanner(dataDirectory, QString(), QStringLiteral("notice"),
                      QStringLiteral("xmatic"),
                      QCoreApplication::translate(
                          "PushWake",
                          "Open xmatic once to finish setting up push notifications."),
                      false, 0);
}
