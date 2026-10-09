#include "behavioursettings.h"

#include "appsettings.h"

#include <QDir>
#include <QFileInfo>
#include <QSettings>

// Same file as the bridge's settings; this class owns "behaviour".

static QString knownOrder(const QString &order)
{
    return order == QLatin1String("name") ? order : QStringLiteral("activity");
}

BehaviourSettings::BehaviourSettings(QObject *parent)
    : QObject(parent)
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    settings.beginGroup(QStringLiteral("behaviour"));
    m_showJoinLeaves = settings.value(QStringLiteral("showJoinLeaves"), false).toBool();
    m_showDisplayNameChanges =
        settings.value(QStringLiteral("showDisplayNameChanges"), false).toBool();
    m_showAvatarChanges = settings.value(QStringLiteral("showAvatarChanges"), false).toBool();
    m_roomOrder = knownOrder(settings.value(QStringLiteral("roomOrder")).toString());
    m_unreadFirst = settings.value(QStringLiteral("unreadFirst"), false).toBool();
    m_spaceOrder = knownOrder(settings.value(QStringLiteral("spaceOrder")).toString());
}

void BehaviourSettings::store(const QString &key, const QVariant &value)
{
    const QString path = appSettingsPath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSettings settings(path, QSettings::IniFormat);
    settings.setValue(QStringLiteral("behaviour/") + key, value);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not save a behaviour setting (status %d)",
                 static_cast<int>(settings.status()));
    }
    emit changed();
}

void BehaviourSettings::setShowJoinLeaves(bool shown)
{
    if (shown != m_showJoinLeaves) {
        m_showJoinLeaves = shown;
        store(QStringLiteral("showJoinLeaves"), shown);
    }
}

void BehaviourSettings::setShowDisplayNameChanges(bool shown)
{
    if (shown != m_showDisplayNameChanges) {
        m_showDisplayNameChanges = shown;
        store(QStringLiteral("showDisplayNameChanges"), shown);
    }
}

void BehaviourSettings::setShowAvatarChanges(bool shown)
{
    if (shown != m_showAvatarChanges) {
        m_showAvatarChanges = shown;
        store(QStringLiteral("showAvatarChanges"), shown);
    }
}

void BehaviourSettings::setRoomOrder(const QString &order)
{
    const QString known = knownOrder(order);
    if (known != m_roomOrder) {
        m_roomOrder = known;
        store(QStringLiteral("roomOrder"), known);
    }
}

void BehaviourSettings::setUnreadFirst(bool first)
{
    if (first != m_unreadFirst) {
        m_unreadFirst = first;
        store(QStringLiteral("unreadFirst"), first);
    }
}

void BehaviourSettings::setSpaceOrder(const QString &order)
{
    const QString known = knownOrder(order);
    if (known != m_spaceOrder) {
        m_spaceOrder = known;
        store(QStringLiteral("spaceOrder"), known);
    }
}
