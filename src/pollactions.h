#ifndef POLLACTIONS_H
#define POLLACTIONS_H

#include <QJsonObject>
#include <QObject>
#include <QStringList>

/// The poll commands, as QML calls them: `matrix.polls`. Knows nothing about
/// the bridge - it hands a finished command up, the bridge sends it. Answers
/// arrive as a diff on the poll's own row, so nothing here waits for one.
class PollActions : public QObject
{
    Q_OBJECT

public:
    explicit PollActions(QObject *parent = nullptr);

    /// Puts a poll into the open room. `undisclosed` keeps the counts closed
    /// until it ends, `maxSelections` is how many answers one vote may carry.
    Q_INVOKABLE void create(const QString &question, const QStringList &answers,
                            bool undisclosed, int maxSelections);

    /// Votes, or changes the vote: `answers` is the whole selection, because
    /// only the latest response per person counts.
    Q_INVOKABLE void vote(const QString &eventId, const QStringList &answers);

    /// Ends a poll. `text` is what clients without poll support show.
    Q_INVOKABLE void end(const QString &eventId, const QString &text);

    /// The core refused a command; `command` is which of the three.
    void reportFailure(const QString &command);

    /// The send queue gave a vote up. The tick stays on the row - a vote is an
    /// aggregation, not a row the SDK could mark - so the page has to say it.
    void reportVoteFailed(const QString &eventId);

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void failed(const QString &command);
    void voteFailed(const QString &eventId);
};

#endif // POLLACTIONS_H
