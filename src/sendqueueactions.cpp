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
    // Another room's answer.
    if (data.value(QStringLiteral("roomId")).toString() != m_roomId) {
        return;
    }
    // Emitted on every answer: the banner waits for one.
    m_stuck = data.value(QStringLiteral("rows")).toArray().toVariantList();
    emit stuckChanged();
    if (data.value(QStringLiteral("reload")).toBool()) {
        emit reloadRequested();
    }
}

void SendQueueActions::reportRoomStuck(const QString &roomId)
{
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
