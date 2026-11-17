#include "banlist.h"

#include <QJsonArray>

static const QString List = QStringLiteral("bans.list");
static const QString Add = QStringLiteral("bans.add");
static const QString Lift = QStringLiteral("bans.lift");

BanList::BanList(QObject *parent)
    : QObject(parent)
{
}

void BanList::load(const QString &roomId)
{
    if (roomId != m_roomId) {
        m_entries.clear();
        m_canBan = false;
        m_loaded = false;
    }
    m_roomId = roomId;
    m_loading = true;
    m_loadError.clear();
    emit changed();

    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    emit commandReady(List, arguments);
}

void BanList::ban(const QString &roomId, const QString &userId, const QString &reason)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("userId"), userId.trimmed());
    arguments.insert(QStringLiteral("reason"), reason.trimmed());
    emit commandReady(Add, arguments);
}

void BanList::unban(const QString &roomId, const QString &userId)
{
    QJsonObject arguments;
    arguments.insert(QStringLiteral("roomId"), roomId);
    arguments.insert(QStringLiteral("userId"), userId);
    emit commandReady(Lift, arguments);
}

void BanList::sent(quint64 id, const QString &command, const QJsonObject &arguments)
{
    const QString roomId = arguments.value(QStringLiteral("roomId")).toString();
    if (command == List) {
        // Unsent: the bridge has said why; the page must not spin for good.
        if (id == 0) {
            m_loading = false;
            emit changed();
        }
        m_loadId = id;
        if (id != 0) {
            m_loads.insert(id);
        }
        return;
    }
    if (id == 0) {
        return;
    }
    m_actions.insert(id, Pending { command, roomId,
                                   arguments.value(QStringLiteral("userId")).toString() });
    emit busyChanged();
}

bool BanList::deliver(quint64 id, const QJsonObject &data)
{
    if (m_loads.remove(id)) {
        // Superseded by a later load, perhaps for another room.
        if (id != m_loadId) {
            return true;
        }
        m_loadId = 0;
        m_entries = data.value(QStringLiteral("bans")).toArray().toVariantList();
        m_canBan = data.value(QStringLiteral("canBan")).toBool();
        m_loaded = true;
        m_loading = false;
        m_loadError.clear();
        emit changed();
        return true;
    }
    if (!m_actions.contains(id)) {
        return false;
    }
    const Pending pending = m_actions.take(id);
    emit busyChanged();

    const QString userId = data.value(QStringLiteral("userId")).toString();
    if (pending.command == Add) {
        if (pending.roomId == m_roomId) {
            removeEntry(userId);
            m_entries.append(data.value(QStringLiteral("row")).toObject().toVariantMap());
            emit changed();
        }
        emit banned(pending.roomId, userId);
    } else {
        if (pending.roomId == m_roomId) {
            removeEntry(userId);
            emit changed();
        }
        emit unbanned(pending.roomId, userId);
    }
    return true;
}

bool BanList::reportFailure(quint64 id, const QString &error)
{
    if (m_loads.remove(id)) {
        if (id != m_loadId) {
            return true;
        }
        m_loadId = 0;
        m_loading = false;
        m_loadError = shown(error);
        emit changed();
        return true;
    }
    if (!m_actions.contains(id)) {
        return false;
    }
    const Pending pending = m_actions.take(id);
    emit busyChanged();
    emit failed(pending.command == Add ? QStringLiteral("ban") : QStringLiteral("unban"),
                pending.roomId, shown(error));
    return true;
}

void BanList::clear()
{
    m_roomId.clear();
    m_entries.clear();
    m_canBan = false;
    m_loaded = false;
    m_loading = false;
    m_loadError.clear();
    m_loadId = 0;
    m_loads.clear();
    m_actions.clear();
    emit changed();
    emit busyChanged();
}

/// The core's own refusals, by their exact text (core/src/bans.rs).
QString BanList::shown(const QString &error)
{
    if (error == QLatin1String("not a Matrix address")) {
        return tr("This is not a valid Matrix address. It looks like @name:server.");
    }
    if (error == QLatin1String("you cannot ban yourself")) {
        return tr("You cannot ban yourself.");
    }
    if (error == QLatin1String("the reason is too long")) {
        return tr("The reason is too long.");
    }
    return error;
}

void BanList::removeEntry(const QString &userId)
{
    for (int i = m_entries.size() - 1; i >= 0; --i) {
        if (m_entries.at(i).toMap().value(QStringLiteral("userId")).toString() == userId) {
            m_entries.removeAt(i);
        }
    }
}
