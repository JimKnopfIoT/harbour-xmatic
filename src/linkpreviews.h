#ifndef LINKPREVIEWS_H
#define LINKPREVIEWS_H

#include <QElapsedTimer>
#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QVariantMap>

/// Link previews, as QML reads them: `matrix.linkPreviews`. Asks the core once
/// per address and keeps the answer; the bridge only carries the command out
/// and the reply back in. Whether a link may be previewed at all is decided
/// where the row is drawn - this object answers what it is asked.
class LinkPreviews : public QObject
{
    Q_OBJECT

public:
    explicit LinkPreviews(QObject *parent = nullptr);

    /// The preview for this address, or an empty map while it is being fetched.
    /// Asking is what starts the fetch; `previewReady` says when it landed.
    Q_INVOKABLE QVariantMap preview(const QString &url);

    /// The core's answer to `link.preview`.
    void deliver(const QJsonObject &data);

    /// Drops everything remembered: with the media cache, and on sign-out
    /// whatever the media setting says.
    void clear();

signals:
    void commandReady(const QString &command, const QJsonObject &arguments);
    void previewReady(const QString &url, const QVariantMap &preview);
    /// A "could not ask" has waited its minute; a card still showing may ask again.
    void retryDue(const QString &url);

private:
    QHash<QString, QVariantMap> m_known;
    /// Addresses asked and not yet answered, with when: an ask the core never
    /// answers (a stalled command) must not block the address for good.
    QHash<QString, qint64> m_asked;
    /// Addresses the core could not ask about, and when. Asked again after a
    /// pause rather than remembered as "nothing there".
    QHash<QString, qint64> m_failedAt;
    /// "Could not ask" per address; past the bound the address is given up
    /// for the session, or a dead page is asked about every minute for ever.
    QHash<QString, int> m_failures;
    QElapsedTimer m_clock;
};

#endif // LINKPREVIEWS_H
