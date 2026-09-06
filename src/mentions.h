#ifndef MENTIONS_H
#define MENTIONS_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QVariantList>

class QTimer;

/// Who can be mentioned while `@` is being typed: `matrix.mentions`. Asks the
/// core through the bridge and holds the answer for one query at a time. What
/// a mention *is* lives in core/src/mention.rs; this only carries.
class Mentions : public QObject
{
    Q_OBJECT

    Q_PROPERTY(QVariantList candidates READ candidates NOTIFY candidatesChanged)
    /// The room the standing list belongs to. Two composers can be alive at
    /// once (a room and its thread), and each has to know whose answer this is.
    Q_PROPERTY(QString roomId READ roomId NOTIFY candidatesChanged)

public:
    explicit Mentions(QObject *parent = nullptr);

    QVariantList candidates() const { return m_candidates; }
    QString roomId() const { return m_roomId; }

    /// Asks for the candidates matching `query` in that room. Debounced: this
    /// runs on every keystroke, and a command per letter is a command too many.
    Q_INVOKABLE void search(const QString &roomId, const QString &query);

    /// Closes the list without asking anything.
    Q_INVOKABLE void clear();

    /// The member page's "Mention" action. Nothing is inserted here - the room
    /// page that stays does that, once it is on top again.
    Q_INVOKABLE void requestInsert(const QString &roomId, const QString &userId,
                                   const QString &displayName);

    /// The core's answer to `mention.candidates`.
    void deliver(const QJsonObject &data);

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void candidatesChanged();
    void insertRequested(const QString &roomId, const QString &userId,
                         const QString &displayName);

private:
    void ask();

    QVariantList m_candidates;
    QString m_roomId;
    /// What was asked for last. An answer that names another query is one the
    /// picker has already moved past.
    QString m_query;
    QString m_pendingRoomId;
    QString m_pendingQuery;
    QTimer *m_debounce = nullptr;
};

#endif // MENTIONS_H
