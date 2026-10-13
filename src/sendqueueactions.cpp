#include "sendqueueactions.h"

#include <QJsonArray>

SendQueueActions::SendQueueActions(QObject *parent)
    : QObject(parent)
{
}

void SendQueueActions::refresh()
{
    if (m_roomId.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), m_roomId);
    emit commandReady(QStringLiteral("queue.stuck"), arguments);
}

void SendQueueActions::retry(const QString &txnId)
{
    if (m_roomId.isEmpty() || txnId.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), m_roomId);
    arguments.insert(QStringLiteral("txnId"), txnId);
    emit commandReady(QStringLiteral("queue.retry"), arguments);
}

void SendQueueActions::discard(const QString &txnId)
{
    if (m_roomId.isEmpty() || txnId.isEmpty()) {
        return;
    }
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), m_roomId);
    arguments.insert(QStringLiteral("txnId"), txnId);
    emit commandReady(QStringLiteral("queue.discard"), arguments);
}

void SendQueueActions::setRoom(const QString &roomId)
{
    if (roomId == m_roomId) {
        return;
    }
    m_roomId = roomId;
    if (!m_stuck.isEmpty()) {
        m_stuck.clear();
        emit stuckChanged();
    }
    refresh();
}

void SendQueueActions::reportStuck(const QJsonObject &data)
{
    const QString roomId = data.value(QStringLiteral("roomId")).toString();
    const QJsonArray rows = data.value(QStringLiteral("rows")).toArray();
    markRoom(roomId, !rows.isEmpty());
    // Another room's answer.
    if (roomId != m_roomId) {
        return;
    }
    // Emitted on every answer: the banner waits for one.
    m_stuck = rows.toVariantList();
    emit stuckChanged();
    if (data.value(QStringLiteral("reload")).toBool()) {
        emit reloadRequested();
    }
}

void SendQueueActions::reportRoomStuck(const QString &roomId)
{
    markRoom(roomId, true);
    if (roomId == m_roomId) {
        refresh();
    }
}

void SendQueueActions::reportDone(const QJsonObject &data)
{
    if (data.value(QStringLiteral("reload")).toBool()
        && data.value(QStringLiteral("roomId")).toString() == m_roomId) {
        emit reloadRequested();
    }
    refresh();
}

void SendQueueActions::reportUnanswered()
{
    emit stuckChanged();
}

void SendQueueActions::reportFailure(const QString &error)
{
    emit failed(error);
    refresh();
}

void SendQueueActions::forgetRooms()
{
    if (m_stuckRooms.isEmpty()) {
        return;
    }
    m_stuckRooms.clear();
    ++m_roomsRevision;
    emit roomsChanged();
}

void SendQueueActions::markRoom(const QString &roomId, bool stuck)
{
    if (roomId.isEmpty() || m_stuckRooms.contains(roomId) == stuck) {
        return;
    }
    if (stuck) {
        m_stuckRooms.insert(roomId);
    } else {
        m_stuckRooms.remove(roomId);
    }
    ++m_roomsRevision;
    emit roomsChanged();
}
