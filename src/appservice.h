#ifndef APPSERVICE_H
#define APPSERVICE_H

#include <QDBusAbstractAdaptor>
#include <QObject>

/// The app's whole outside surface: every process on the bus can reach it, so it
/// stays small. A link is a stranger's string - filtered here, decided in
/// `MatrixLinks.js`, and never acted on beyond showing a room.
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
    /// Takes a link from outside. Answers whether it was one this app can mean
    /// anything by; anything else is dropped without a trace, not logged - a
    /// matrix.to link carries a room address and often a user id.
    bool requestLink(const QString &link);

    /// A link handed over at start-up, waiting for the UI to be there to take
    /// it. Empty once it has been.
    Q_INVOKABLE QString takePendingLink();

    /// The app asking for its own window, for a ringing call: a notification
    /// banner is gone in a moment, and a call has to be answerable.
    Q_INVOKABLE void raiseWindow() { emit raiseRequested(); }

signals:
    /// Bring the window forward.
    void raiseRequested();
    /// Bring the window forward and open the notified room.
    void notifiedRoomRequested();
    /// A Matrix link arrived from outside; the UI decides what it means.
    void linkRequested(const QString &link);

private:
    QString m_pendingLink;
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
    // All of them answer with an empty reply rather than Q_NOREPLY: the instance
    // handover calls blocking, and a silent method would make it wait out the
    // timeout.
    void activate();
    void openNotified();
    /// The link handler's way in, and the one method here that takes something
    /// from the caller. What it accepts is decided in AppService::requestLink.
    void openUrl(const QString &link);

private:
    AppService *m_service;
};

#endif // APPSERVICE_H
