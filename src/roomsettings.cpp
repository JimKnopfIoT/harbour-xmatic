#include "roomsettings.h"

static const QString Prefix = QStringLiteral("roomSettings.");

RoomSettings::RoomSettings(QObject *parent)
    : QObject(parent)
{
}

void RoomSettings::load(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    emit commandReady(Prefix + QStringLiteral("load"), arguments);
}

void RoomSettings::setName(const QString &roomId, const QString &name)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("name"), name);
    emit commandReady(Prefix + QStringLiteral("setName"), arguments);
}

void RoomSettings::setTopic(const QString &roomId, const QString &topic)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("topic"), topic);
    emit commandReady(Prefix + QStringLiteral("setTopic"), arguments);
}

void RoomSettings::setAvatar(const QString &roomId, const QString &path)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("path"), path);
    emit commandReady(Prefix + QStringLiteral("setAvatar"), arguments);
}

void RoomSettings::removeAvatar(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    emit commandReady(Prefix + QStringLiteral("removeAvatar"), arguments);
}

void RoomSettings::sent(quint64 id, const QString &command, const QJsonObject &arguments)
{
    if (id == 0) {
        return;
    }
    const QString verb = command.mid(Prefix.size());
    Pending pending;
    pending.roomId = arguments.value(QStringLiteral("roomId")).toString();
    pending.field = verb == QLatin1String("load") ? QString()
                  : verb == QLatin1String("setName") ? QStringLiteral("name")
                  : verb == QLatin1String("setTopic") ? QStringLiteral("topic")
                  : QStringLiteral("avatar");
    m_pending.insert(id, pending);
}

bool RoomSettings::deliver(quint64 id, const QJsonObject &data)
{
    if (!m_pending.contains(id)) {
        return false;
    }
    const Pending pending = m_pending.take(id);
    if (pending.field.isEmpty()) {
        emit loaded(data.toVariantMap());
        return true;
    }
    const QString value = data.value(pending.field).toString();
    const QJsonObject row = data.value(QStringLiteral("row")).toObject();
    emit saved(pending.roomId, pending.field, value,
               row.value(QStringLiteral("name")).toString());
    if (!row.isEmpty()) {
        emit rowChanged(pending.roomId, row);
    }
    return true;
}

bool RoomSettings::reportFailure(quint64 id, const QString &error)
{
    if (!m_pending.contains(id)) {
        return false;
    }
    const Pending pending = m_pending.take(id);
    emit failed(pending.roomId, pending.field, error);
    return true;
}
