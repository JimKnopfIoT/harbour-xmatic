#include "linkpreviews.h"

#include <QTimer>

/// How long a "could not ask" stands before the same address is tried again,
/// and how long an unanswered ask holds the address.
static const qint64 RetryAfterMs = 60 * 1000;
static const qint64 AskTimeoutMs = 150 * 1000;
static const int MaxFailures = 3;
/// The nudge lands after the pause, not on it: a coarse timer may run early.
static const int NudgeMs = static_cast<int>(RetryAfterMs) + 2000;

LinkPreviews::LinkPreviews(QObject *parent)
    : QObject(parent)
{
    m_clock.start();
}

QVariantMap LinkPreviews::preview(const QString &url)
{
    if (url.isEmpty()) {
        return QVariantMap();
    }
    const auto known = m_known.constFind(url);
    if (known != m_known.constEnd()) {
        return *known;
    }
    // One request per address; a row is rebound every time it scrolls into view.
    // A failed ask is repeated, but not on every rebind.
    const qint64 failedAt = m_failedAt.value(url, -1);
    if (failedAt >= 0 && m_clock.elapsed() - failedAt < RetryAfterMs) {
        return QVariantMap();
    }
    const qint64 askedAt = m_asked.value(url, -1);
    if (askedAt < 0 || m_clock.elapsed() - askedAt > AskTimeoutMs) {
        m_asked.insert(url, m_clock.elapsed());
        QJsonObject arguments;
        arguments.insert(QStringLiteral("url"), url);
        emit commandReady(QStringLiteral("link.preview"), arguments);
    }
    return QVariantMap();
}

void LinkPreviews::deliver(const QJsonObject &data)
{
    const QString url = data.value(QStringLiteral("url")).toString();
    if (url.isEmpty()) {
        return;
    }
    m_asked.remove(url);
    // "Could not ask" is not "nothing there": the first is tried again later,
    // the second is remembered so the card stops asking for a silent page.
    if (data.value(QStringLiteral("retry")).toBool()) {
        const int failures = m_failures.value(url) + 1;
        if (failures >= MaxFailures) {
            // Given up: remembered as nothing there, so the card stops asking.
            m_failures.remove(url);
            m_failedAt.remove(url);
            const QVariantMap preview = data.toVariantMap();
            m_known.insert(url, preview);
            emit previewReady(url, preview);
            return;
        }
        m_failures.insert(url, failures);
        m_failedAt.insert(url, m_clock.elapsed());
        QTimer::singleShot(NudgeMs, this, [this, url] {
            if (m_failedAt.contains(url)) {
                emit retryDue(url);
            }
        });
        return;
    }
    m_failedAt.remove(url);
    m_failures.remove(url);
    const QVariantMap preview = data.toVariantMap();
    m_known.insert(url, preview);
    emit previewReady(url, preview);
}

void LinkPreviews::clear()
{
    m_known.clear();
    m_asked.clear();
    m_failedAt.clear();
    m_failures.clear();
}
