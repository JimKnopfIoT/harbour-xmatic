#include "pushrules.h"

static const QString Prefix = QStringLiteral("pushRules.");

PushRules::PushRules(QObject *parent)
    : QObject(parent)
{
}

void PushRules::load()
{
    emit commandReady(Prefix + QStringLiteral("get"), QJsonObject());
}

void PushRules::set(const QString &category, const QString &level)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("category"), category);
    arguments.insert(QStringLiteral("level"), level);
    emit commandReady(Prefix + QStringLiteral("set"), arguments);
}

void PushRules::sent(quint64 id, const QString &command)
{
    if (id == 0) {
        return;
    }
    const bool wasBusy = busy();
    m_pending.insert(id, command == Prefix + QStringLiteral("set"));
    if (!wasBusy) {
        emit busyChanged();
    }
}

void PushRules::take(quint64 id)
{
    m_pending.remove(id);
    if (m_pending.isEmpty()) {
        emit busyChanged();
    }
}

// Always signalled: a pick that failed must snap back to the levels last read.
void PushRules::markStale()
{
    m_stale = true;
    emit levelsChanged();
}

bool PushRules::deliver(quint64 id, const QJsonObject &data)
{
    if (!m_pending.contains(id)) {
        return false;
    }
    take(id);
    // A double: exact far beyond any count of answers one run gives.
    const quint64 sequence = static_cast<quint64>(data.value(QStringLiteral("seq")).toDouble());
    const bool readBack = data.contains(QStringLiteral("levels"));
    if (readBack && sequence > m_sequence) {
        m_sequence = sequence;
        m_levels = data.value(QStringLiteral("levels")).toObject().toVariantMap();
        m_loaded = true;
        m_stale = false;
        emit levelsChanged();
    }
    const QString error = data.value(QStringLiteral("error")).toString();
    if (!error.isEmpty()) {
        qWarning("xmatic: push rules: %s", qPrintable(error));
        if (!readBack) {
            // No older read may now stand in for what this change left behind.
            m_sequence = qMax(m_sequence, sequence);
            markStale();
        }
        emit failed(error, data.value(QStringLiteral("failure")).toString());
    }
    return true;
}

bool PushRules::reportFailure(quint64 id, const QString &error)
{
    if (!m_pending.contains(id)) {
        return false;
    }
    const bool writing = m_pending.value(id);
    take(id);
    qWarning("xmatic: push rules: %s", qPrintable(error));
    markStale();
    emit failed(error, writing ? QStringLiteral("write") : QStringLiteral("read"));
    return true;
}

void PushRules::clear()
{
    const bool wasBusy = busy();
    m_pending.clear();
    m_levels.clear();
    m_loaded = false;
    m_stale = false;
    m_sequence = 0;
    emit levelsChanged();
    if (wasBusy) {
        emit busyChanged();
    }
}
