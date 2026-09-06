#include "mentions.h"

#include <QJsonArray>
#include <QTimer>

/// Long enough that a word typed at speed asks once, short enough that the
/// list is there by the time the finger stops.
static const int DebounceMs = 180;

Mentions::Mentions(QObject *parent)
    : QObject(parent)
{
    m_debounce = new QTimer(this);
    m_debounce->setSingleShot(true);
    m_debounce->setInterval(DebounceMs);
    connect(m_debounce, &QTimer::timeout, this, &Mentions::ask);
}

void Mentions::search(const QString &roomId, const QString &query)
{
    if (roomId.isEmpty()) {
        clear();
        return;
    }
    m_pendingRoomId = roomId;
    m_pendingQuery = query;
    m_debounce->start();
}

void Mentions::ask()
{
    m_roomId = m_pendingRoomId;
    m_query = m_pendingQuery;
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), m_roomId);
    arguments.insert(QStringLiteral("query"), m_query);
    emit commandReady(QStringLiteral("mention.candidates"), arguments);
}

void Mentions::clear()
{
    m_debounce->stop();
    m_pendingRoomId.clear();
    m_pendingQuery.clear();
    m_query.clear();
    if (m_candidates.isEmpty() && m_roomId.isEmpty()) {
        return;
    }
    m_candidates.clear();
    m_roomId.clear();
    emit candidatesChanged();
}

void Mentions::requestInsert(const QString &roomId, const QString &userId,
                             const QString &displayName)
{
    if (roomId.isEmpty() || userId.isEmpty()) {
        return;
    }
    emit insertRequested(roomId, userId, displayName);
}

void Mentions::deliver(const QJsonObject &data)
{
    // The query travels out and back: a command has no id on this side, and an
    // answer to what was typed two letters ago must not reopen the list.
    if (data.value(QStringLiteral("roomId")).toString() != m_roomId
            || data.value(QStringLiteral("query")).toString() != m_query) {
        return;
    }
    m_candidates = data.value(QStringLiteral("candidates")).toArray().toVariantList();
    emit candidatesChanged();
}
