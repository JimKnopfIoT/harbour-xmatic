#include "leftrooms.h"

#include <QJsonArray>

static const QString Prefix = QStringLiteral("leftRooms.");

LeftRooms::LeftRooms(QObject *parent)
    : QObject(parent)
{
}

void LeftRooms::setCount(int count)
{
    if (count != m_count) {
        m_count = count;
        emit countChanged();
    }
}

void LeftRooms::load()
{
    emit commandReady(Prefix + QStringLiteral("list"), QJsonObject());
}

void LeftRooms::forget(const QString &roomId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    emit commandReady(Prefix + QStringLiteral("forget"), arguments);
}

void LeftRooms::sent(quint64 id, const QString &command, const QJsonObject &arguments)
{
    Q_UNUSED(command)
    if (id != 0) {
        m_pending.insert(id, arguments.value(QStringLiteral("roomId")).toString());
    }
}

bool LeftRooms::deliver(quint64 id, const QJsonObject &data)
{
    if (!m_pending.contains(id)) {
        return false;
    }
    const QString roomId = m_pending.take(id);
    if (roomId.isEmpty()) {
        const QVariantList rooms = data.value(QStringLiteral("rooms")).toArray().toVariantList();
        setCount(rooms.size());
        emit loaded(rooms);
        return true;
    }
    // Forgetting changes no row the chat list shows, so no diff brings the count.
    setCount(data.value(QStringLiteral("left")).toInt(m_count));
    emit forgotten(roomId);
    return true;
}

bool LeftRooms::reportFailure(quint64 id, const QString &error)
{
    if (!m_pending.contains(id)) {
        return false;
    }
    emit failed(m_pending.take(id), error);
    return true;
}
