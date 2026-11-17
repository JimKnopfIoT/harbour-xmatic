#ifndef PUSHRULES_H
#define PUSHRULES_H

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QVariantMap>

/// `matrix.pushRules`: the account's notification level per kind of message,
/// `off`, `on`, `noisy` or `mixed`. The rules live in the account, so every client follows them.
class PushRules : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QVariantMap levels READ levels NOTIFY levelsChanged)
    Q_PROPERTY(bool loaded READ loaded NOTIFY levelsChanged)
    Q_PROPERTY(bool stale READ stale NOTIFY levelsChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)

public:
    explicit PushRules(QObject *parent = nullptr);

    QVariantMap levels() const { return m_levels; }
    bool loaded() const { return m_loaded; }
    bool stale() const { return m_stale; }
    bool busy() const { return !m_pending.isEmpty(); }

    Q_INVOKABLE void load();
    Q_INVOKABLE void set(const QString &category, const QString &level);

    void sent(quint64 id, const QString &command);
    bool deliver(quint64 id, const QJsonObject &data);
    bool reportFailure(quint64 id, const QString &error);
    /// The next account must not see this one's levels.
    void clear();

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void levelsChanged();
    void busyChanged();
    /// `kind`: `read` (the rules could not be read), `write` (the server
    /// refused a change) or `late` (a change went unanswered).
    void failed(const QString &error, const QString &kind);

private:
    void take(quint64 id);
    /// No read came back since the last failure; the levels shown are the last known.
    void markStale();

    QVariantMap m_levels;
    bool m_loaded = false;
    bool m_stale = false;
    /// Command id → whether it writes.
    QHash<quint64, bool> m_pending;
    /// The core's answer order; an older answer arriving late is dropped.
    quint64 m_sequence = 0;
};

#endif // PUSHRULES_H
