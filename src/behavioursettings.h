#ifndef BEHAVIOURSETTINGS_H
#define BEHAVIOURSETTINGS_H

#include <QObject>
#include <QString>
#include <QVariant>

/// Which room events the timeline shows, and how the room and space lists sort.
/// Joins always show; leaves and profile changes are off by default.
class BehaviourSettings : public QObject
{
    Q_OBJECT

    Q_PROPERTY(bool showLeaves READ showLeaves WRITE setShowLeaves NOTIFY changed)
    Q_PROPERTY(bool showDisplayNameChanges READ showDisplayNameChanges
               WRITE setShowDisplayNameChanges NOTIFY changed)
    Q_PROPERTY(bool showAvatarChanges READ showAvatarChanges WRITE setShowAvatarChanges
               NOTIFY changed)
    /// "activity" (the SDK's order) or "name".
    Q_PROPERTY(QString roomOrder READ roomOrder WRITE setRoomOrder NOTIFY changed)
    Q_PROPERTY(bool unreadFirst READ unreadFirst WRITE setUnreadFirst NOTIFY changed)
    Q_PROPERTY(QString spaceOrder READ spaceOrder WRITE setSpaceOrder NOTIFY changed)
    /// This device only: a notification without a tone is shown at all.
    Q_PROPERTY(bool showSilentNotifications READ showSilentNotifications
               WRITE setShowSilentNotifications NOTIFY changed)
    /// This device only, message banners: pop-up, LED, tone and vibration, as the rule has them.
    /// Off: no notification at all, not even an entry in the event view.
    Q_PROPERTY(bool notificationEntry READ notificationEntry WRITE setNotificationEntry
               NOTIFY changed)
    Q_PROPERTY(bool notificationPopup READ notificationPopup WRITE setNotificationPopup
               NOTIFY changed)
    Q_PROPERTY(bool notificationLed READ notificationLed WRITE setNotificationLed NOTIFY changed)
    Q_PROPERTY(bool notificationSound READ notificationSound WRITE setNotificationSound
               NOTIFY changed)
    Q_PROPERTY(bool notificationVibration READ notificationVibration
               WRITE setNotificationVibration NOTIFY changed)
    /// "all", or "quiet": the three above change only what the rule sends without tone.
    Q_PROPERTY(QString notificationScope READ notificationScope WRITE setNotificationScope
               NOTIFY changed)
    /// "all", "notifying" (what the rules notify) or "none": what the unread numbers count.
    Q_PROPERTY(QString unreadCount READ unreadCount WRITE setUnreadCount NOTIFY changed)

public:
    /// The device-only notification switches, read together.
    struct DeviceNotifications {
        bool showSilent = true;
        bool entry = true;
        bool popup = true;
        bool led = true;
        bool sound = true;
        bool vibration = true;
        bool quietOnly = false;
        QString unreadCount = QStringLiteral("all");
    };

    explicit BehaviourSettings(QObject *parent = nullptr);

    bool showLeaves() const { return m_showLeaves; }
    bool showDisplayNameChanges() const { return m_showDisplayNameChanges; }
    bool showAvatarChanges() const { return m_showAvatarChanges; }
    QString roomOrder() const { return m_roomOrder; }
    bool unreadFirst() const { return m_unreadFirst; }
    QString spaceOrder() const { return m_spaceOrder; }
    bool showSilentNotifications() const { return m_device.showSilent; }
    bool notificationEntry() const { return m_device.entry; }
    bool notificationPopup() const { return m_device.popup; }
    bool notificationLed() const { return m_device.led; }
    bool notificationSound() const { return m_device.sound; }
    bool notificationVibration() const { return m_device.vibration; }
    QString notificationScope() const;
    QString unreadCount() const { return m_device.unreadCount; }
    /// Read from the file each time: the wake process has no instance.
    static DeviceNotifications deviceNotifications();

    void setShowLeaves(bool shown);
    void setShowDisplayNameChanges(bool shown);
    void setShowAvatarChanges(bool shown);
    void setRoomOrder(const QString &order);
    void setUnreadFirst(bool first);
    void setSpaceOrder(const QString &order);
    void setShowSilentNotifications(bool shown);
    void setNotificationEntry(bool shown);
    void setNotificationPopup(bool shown);
    void setNotificationLed(bool shown);
    void setNotificationSound(bool shown);
    void setNotificationVibration(bool shown);
    void setNotificationScope(const QString &scope);
    void setUnreadCount(const QString &count);

signals:
    void changed();

private:
    void store(const QString &key, const QVariant &value);
    void storeSwitch(bool &field, bool value, const char *key);

    bool m_showLeaves = false;
    bool m_showDisplayNameChanges = false;
    bool m_showAvatarChanges = false;
    QString m_roomOrder;
    bool m_unreadFirst = false;
    QString m_spaceOrder;
    DeviceNotifications m_device;
};

#endif // BEHAVIOURSETTINGS_H
