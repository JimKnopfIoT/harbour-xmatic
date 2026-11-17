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

/// Every switch defaults to on, which is how notifications behaved before them.
static BehaviourSettings::DeviceNotifications readDevice(QSettings &settings)
{
    BehaviourSettings::DeviceNotifications device;
    const auto read = [&settings](const char *key) {
        return settings.value(QStringLiteral("behaviour/") + QLatin1String(key), true).toBool();
    };
    device.showSilent = read("showSilentNotifications");
    device.entry = read("notificationEntry");
    device.popup = read("notificationPopup");
    device.led = read("notificationLed");
    device.sound = read("notificationSound");
    device.vibration = read("notificationVibration");
    device.quietOnly = settings.value(QStringLiteral("behaviour/notificationQuietOnly"), false)
                               .toBool();
    const QString count = settings.value(QStringLiteral("behaviour/unreadCount")).toString();
    if (count == QLatin1String("notifying") || count == QLatin1String("none")) {
        device.unreadCount = count;
    }
    return device;
}

BehaviourSettings::BehaviourSettings(QObject *parent)
    : QObject(parent)
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    settings.beginGroup(QStringLiteral("behaviour"));
    // One switch governed joins and leaves; joins show always now, so its choice
    // carries over to leaves alone, once.
    const QString leaves = QStringLiteral("showLeaves");
    const QString joinLeaves = QStringLiteral("showJoinLeaves");
    if (settings.contains(leaves)) {
        m_showLeaves = settings.value(leaves, false).toBool();
    } else if (settings.contains(joinLeaves)) {
        m_showLeaves = settings.value(joinLeaves, false).toBool();
        settings.setValue(leaves, m_showLeaves);
        settings.remove(joinLeaves);
        settings.sync();
    }
    m_showDisplayNameChanges =
        settings.value(QStringLiteral("showDisplayNameChanges"), false).toBool();
    m_showAvatarChanges = settings.value(QStringLiteral("showAvatarChanges"), false).toBool();
    m_roomOrder = knownOrder(settings.value(QStringLiteral("roomOrder")).toString());
    m_unreadFirst = settings.value(QStringLiteral("unreadFirst"), false).toBool();
    m_spaceOrder = knownOrder(settings.value(QStringLiteral("spaceOrder")).toString());
    settings.endGroup();
    m_device = readDevice(settings);
}

BehaviourSettings::DeviceNotifications BehaviourSettings::deviceNotifications()
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return readDevice(settings);
}

QString BehaviourSettings::notificationScope() const
{
    return m_device.quietOnly ? QStringLiteral("quiet") : QStringLiteral("all");
}

void BehaviourSettings::storeSwitch(bool &field, bool value, const char *key)
{
    if (value != field) {
        field = value;
        store(QLatin1String(key), value);
    }
}

void BehaviourSettings::setShowSilentNotifications(bool shown)
{
    storeSwitch(m_device.showSilent, shown, "showSilentNotifications");
}

void BehaviourSettings::setNotificationEntry(bool shown)
{
    storeSwitch(m_device.entry, shown, "notificationEntry");
}

void BehaviourSettings::setNotificationPopup(bool shown)
{
    storeSwitch(m_device.popup, shown, "notificationPopup");
}

void BehaviourSettings::setNotificationLed(bool shown)
{
    storeSwitch(m_device.led, shown, "notificationLed");
}

void BehaviourSettings::setNotificationSound(bool shown)
{
    storeSwitch(m_device.sound, shown, "notificationSound");
}

void BehaviourSettings::setNotificationVibration(bool shown)
{
    storeSwitch(m_device.vibration, shown, "notificationVibration");
}

void BehaviourSettings::setNotificationScope(const QString &scope)
{
    if (scope == QLatin1String("quiet") || scope == QLatin1String("all")) {
        storeSwitch(m_device.quietOnly, scope == QLatin1String("quiet"), "notificationQuietOnly");
    }
}

void BehaviourSettings::setUnreadCount(const QString &count)
{
    const bool known = count == QLatin1String("all") || count == QLatin1String("notifying")
            || count == QLatin1String("none");
    if (known && count != m_device.unreadCount) {
        m_device.unreadCount = count;
        store(QStringLiteral("unreadCount"), count);
    }
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

void BehaviourSettings::setShowLeaves(bool shown)
{
    if (shown != m_showLeaves) {
        m_showLeaves = shown;
        store(QStringLiteral("showLeaves"), shown);
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
