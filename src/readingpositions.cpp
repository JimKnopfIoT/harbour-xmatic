#include "readingpositions.h"

ReadingPositions::ReadingPositions(QObject *parent)
    : QObject(parent)
{
}

void ReadingPositions::remember(const QString &roomId, const QString &eventId, qreal offset)
{
    if (roomId.isEmpty()) {
        return;
    }
    if (eventId.isEmpty()) {
        m_positions.remove(roomId);
        return;
    }
    Position held;
    held.eventId = eventId;
    held.offset = offset;
    m_positions.insert(roomId, held);
}

QVariantMap ReadingPositions::recall(const QString &roomId) const
{
    const auto held = m_positions.constFind(roomId);
    if (held == m_positions.constEnd()) {
        return QVariantMap();
    }
    QVariantMap answer;
    answer.insert(QStringLiteral("eventId"), held->eventId);
    answer.insert(QStringLiteral("offset"), held->offset);
    return answer;
}

void ReadingPositions::forget(const QString &roomId)
{
    m_positions.remove(roomId);
}

void ReadingPositions::clear()
{
    m_positions.clear();
}
