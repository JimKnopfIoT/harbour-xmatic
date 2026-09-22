#ifndef READINGPOSITIONS_H
#define READINGPOSITIONS_H

#include <QHash>
#include <QObject>
#include <QVariantMap>

/// Where the reader stood in each room, as `positions`. Memory only and for as
/// long as the app runs: an event id is an identifier, and the settings file is
/// not the place for one.
class ReadingPositions : public QObject
{
    Q_OBJECT

public:
    explicit ReadingPositions(QObject *parent = nullptr);

    /// The topmost visible row and how far it sits above the edge. An empty
    /// event id forgets the room instead - a local echo has none.
    Q_INVOKABLE void remember(const QString &roomId, const QString &eventId, qreal offset);

    /// `eventId` and `offset`, or an empty map where nothing is held.
    Q_INVOKABLE QVariantMap recall(const QString &roomId) const;

    Q_INVOKABLE void forget(const QString &roomId);

    /// Signing out ends every room this knew about.
    void clear();

private:
    struct Position {
        QString eventId;
        qreal offset = 0;
    };

    QHash<QString, Position> m_positions;
};

#endif // READINGPOSITIONS_H
