#ifndef APPSERVICE_H
#define APPSERVICE_H

#include <QDBusAbstractAdaptor>
#include <QObject>

/// The app's own D-Bus object, and the whole of its outside surface.
///
/// Two methods, neither of them taking an argument. That is deliberate: this
/// object is reachable by every process on the session bus, so it can only be
/// asked to do things whose effect is already decided inside the app.
///
/// * `activate` raises the window. It is what the launcher's hand-over and the
///   share service call. Nothing implemented it before - the name is owned but
///   the method was not there, so the call answered "unknown method" and a
///   running app stayed in the background.
/// * `openNotified` raises the window and opens the room the standing
///   notification is about. Which room that is comes from the app's own state,
///   never from the caller; without a standing notification it does nothing,
///   and it does it once - the second call finds nothing left to open.
class AppService : public QObject
{
    Q_OBJECT

public:
    explicit AppService(QObject *parent = nullptr);

    /// Registers the object on the session bus. Answers false when the bus is
    /// unavailable, which is not fatal: the app runs, it just cannot be
    /// reached from outside.
    bool publish();

    void requestRaise();
    void requestNotifiedRoom();

signals:
    /// Bring the window forward.
    void raiseRequested();
    /// Bring the window forward and open the notified room.
    void notifiedRoomRequested();
};

/// The interface as the notification and the service file name it. Separate
/// from AppService so the exported method names are exactly these two and
/// nothing else of the object leaks onto the bus.
class AppServiceAdaptor : public QDBusAbstractAdaptor
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.xmatic.xmatic")

public:
    explicit AppServiceAdaptor(AppService *service);

public slots:
    // Both answer with an empty reply rather than Q_NOREPLY: the instance
    // hand-over calls blocking with a two-second timeout, and a method that
    // never answers would make it wait out that timeout for nothing.
    void activate();
    void openNotified();

private:
    AppService *m_service;
};

#endif // APPSERVICE_H
