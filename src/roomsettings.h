#ifndef ROOMSETTINGS_H
#define ROOMSETTINGS_H

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QVariantMap>

/// `matrix.roomSettings`: a room's name, topic and picture.
class RoomSettings : public QObject
{
    Q_OBJECT

public:
    explicit RoomSettings(QObject *parent = nullptr);

    Q_INVOKABLE void load(const QString &roomId);
    Q_INVOKABLE void setName(const QString &roomId, const QString &name);
    Q_INVOKABLE void setTopic(const QString &roomId, const QString &topic);
    Q_INVOKABLE void setAvatar(const QString &roomId, const QString &path);
    Q_INVOKABLE void removeAvatar(const QString &roomId);

    void sent(quint64 id, const QString &command, const QJsonObject &arguments);
    bool deliver(quint64 id, const QJsonObject &data);
    bool reportFailure(quint64 id, const QString &error);

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void loaded(const QVariantMap &settings);
    void saved(const QString &roomId, const QString &field, const QString &value,
               const QString &shownName);
    void failed(const QString &roomId, const QString &field, const QString &error);
    /// No diff is promised for a picture.
    void rowChanged(const QString &roomId, const QJsonObject &fields);

private:
    struct Pending {
        QString roomId;
        QString field;
    };
    QHash<quint64, Pending> m_pending;
};

#endif // ROOMSETTINGS_H
