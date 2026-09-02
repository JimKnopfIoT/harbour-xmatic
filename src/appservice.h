#ifndef APPSERVICE_H
#define APPSERVICE_H

#include <QDBusAbstractAdaptor>
#include <QObject>

/// The app's D-Bus object and its whole outside surface: two methods, neither
/// taking an argument - every process on the bus can reach it.
class AppService : public QObject
{
    Q_OBJECT

public:
    explicit AppService(QObject *parent = nullptr);

    /// Registers the object on the session bus. False where the bus is unavailable,
    /// which is not fatal: the app runs, it just cannot be reached.
    bool publish();

    void requestRaise();
    void requestNotifiedRoom();

    /// The app asking for its own window, for a ringing call: a notification
    /// banner is gone in a moment, and a call has to be answerable.
    Q_INVOKABLE void raiseWindow() { emit raiseRequested(); }

signals:
    /// Bring the window forward.
    void raiseRequested();
    /// Bring the window forward and open the notified room.
    void notifiedRoomRequested();
};

/// The interface as the notification and the service file name it. Separate so
/// nothing else of the object leaks onto the bus.
class AppServiceAdaptor : public QDBusAbstractAdaptor
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.xmatic.xmatic")

public:
    explicit AppServiceAdaptor(AppService *service);

public slots:
    // Both answer with an empty reply rather than Q_NOREPLY: the instance handover
    // calls blocking, and a silent method would make it wait out the timeout.
    void activate();
    void openNotified();

private:
    AppService *m_service;
};

#endif // APPSERVICE_H
