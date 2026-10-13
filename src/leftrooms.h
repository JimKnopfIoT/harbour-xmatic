#ifndef LEFTROOMS_H
#define LEFTROOMS_H

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QVariantList>

/// `matrix.leftRooms`: rooms left or banned from but not forgotten. The chat
/// list hides them, the server counts them until they are forgotten.
class LeftRooms : public QObject
{
    Q_OBJECT
    Q_PROPERTY(int count READ count NOTIFY countChanged)

public:
    explicit LeftRooms(QObject *parent = nullptr);

    int count() const { return m_count; }
    void setCount(int count);

    Q_INVOKABLE void load();
    Q_INVOKABLE void forget(const QString &roomId);

    void sent(quint64 id, const QString &command, const QJsonObject &arguments);
    bool deliver(quint64 id, const QJsonObject &data);
    bool reportFailure(quint64 id, const QString &error);

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void countChanged();
    void loaded(const QVariantList &rooms);
    void forgotten(const QString &roomId);
    void failed(const QString &roomId, const QString &error);

private:
    int m_count = 0;
    /// Command id to room id; empty for a list request.
    QHash<quint64, QString> m_pending;
};

#endif // LEFTROOMS_H
