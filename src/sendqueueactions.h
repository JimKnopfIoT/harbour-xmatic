#ifndef SENDQUEUEACTIONS_H
#define SENDQUEUEACTIONS_H

#include <QJsonObject>
#include <QObject>
#include <QVariantList>

/// `matrix.sendQueue`: the open room's parked requests. The bridge sends.
class SendQueueActions : public QObject
{
    Q_OBJECT
    /// Rows of { txnId, kind, reason, detail }, first the one blocking the rest.
    Q_PROPERTY(QVariantList stuck READ stuck NOTIFY stuckChanged)

public:
    explicit SendQueueActions(QObject *parent = nullptr);

    QVariantList stuck() const { return m_stuck; }

    Q_INVOKABLE void refresh();
    Q_INVOKABLE void retry(const QString &txnId);
    Q_INVOKABLE void discard(const QString &txnId);

    /// The room whose queue is shown; empty when none is open.
    void setRoom(const QString &roomId);
    void reportStuck(const QJsonObject &data);
    /// The core saw a request parked in `roomId`.
    void reportRoomStuck(const QString &roomId);
    /// A retry or discard was answered: the list is asked for again either way.
    void reportDone(const QJsonObject &data);
    void reportFailure(const QString &error);
    /// No answer will come; the page stops waiting.
    void reportUnanswered();

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void stuckChanged();
    /// A removed reaction echo hid a server reaction: the timeline is rebuilt.
    void reloadRequested();
    void failed(const QString &message);

private:
    QString m_roomId;
    QVariantList m_stuck;
};

#endif // SENDQUEUEACTIONS_H
