#include "pollactions.h"

#include <QJsonArray>

PollActions::PollActions(QObject *parent)
    : QObject(parent)
{
}

void PollActions::create(const QString &question, const QStringList &answers,
                         bool undisclosed, int maxSelections)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("question"), question);
    arguments.insert(QStringLiteral("answers"), QJsonArray::fromStringList(answers));
    arguments.insert(QStringLiteral("undisclosed"), undisclosed);
    arguments.insert(QStringLiteral("maxSelections"), qMax(1, maxSelections));
    emit commandReady(QStringLiteral("poll.start"), arguments);
}

void PollActions::vote(const QString &eventId, const QStringList &answers)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    arguments.insert(QStringLiteral("answers"), QJsonArray::fromStringList(answers));
    emit commandReady(QStringLiteral("poll.vote"), arguments);
}

void PollActions::end(const QString &eventId, const QString &text)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("eventId"), eventId);
    arguments.insert(QStringLiteral("text"), text);
    emit commandReady(QStringLiteral("poll.end"), arguments);
}

void PollActions::reportFailure(const QString &command)
{
    emit failed(command);
}

void PollActions::reportVoteFailed(const QString &eventId)
{
    emit voteFailed(eventId);
}
