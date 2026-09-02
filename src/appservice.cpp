#include "appservice.h"

#include <QDBusConnection>

namespace {

// The path the notification's action, the share service and the instance
// handover already use. The name is Sailjail's, claimed on the QML side.
const char *kObjectPath = "/org/xmatic/xmatic";

} // namespace

AppService::AppService(QObject *parent)
    : QObject(parent)
{
    new AppServiceAdaptor(this);
}

bool AppService::publish()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) {
        qWarning("xmatic: no session bus; the app cannot be raised from outside");
        return false;
    }
    if (!bus.registerObject(QString::fromLatin1(kObjectPath), this,
                            QDBusConnection::ExportAdaptors)) {
        qWarning("xmatic: the D-Bus object could not be registered");
        return false;
    }
    return true;
}

void AppService::requestRaise()
{
    emit raiseRequested();
}

void AppService::requestNotifiedRoom()
{
    emit notifiedRoomRequested();
}

AppServiceAdaptor::AppServiceAdaptor(AppService *service)
    : QDBusAbstractAdaptor(service)
    , m_service(service)
{
}

void AppServiceAdaptor::activate()
{
    m_service->requestRaise();
}

void AppServiceAdaptor::openNotified()
{
    m_service->requestNotifiedRoom();
}
