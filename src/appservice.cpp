#include "appservice.h"

#include <QDBusConnection>
#include <QRegularExpression>

namespace {

// What may pass as a link, before anything looks at what it means. Narrow on
// purpose: the app is registered for https as a whole, because that is the only
// way a matrix.to link reaches it, so most of what arrives here is somebody
// else's web address and has to be dropped rather than carried further.
bool looksLikeMatrixLink(const QString &link)
{
    if (link.startsWith(QLatin1String("matrix:"), Qt::CaseInsensitive)) {
        return true;
    }
    // The host has to *be* matrix.to - the same rule MatrixLinks.js follows,
    // where a tracker URL carrying it in a query once passed as internal.
    static const QRegularExpression permalink(
        QStringLiteral("^https?://(www\\.)?matrix\\.to/#/"),
        QRegularExpression::CaseInsensitiveOption);
    return permalink.match(link).hasMatch();
}

} // namespace

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

bool AppService::requestLink(const QString &link)
{
    if (!looksLikeMatrixLink(link)) {
        // Not logged, not answered with anything the caller could read a
        // decision from: a stranger learns nothing about this app from a link.
        return false;
    }
    // Before the window exists there is nobody to tell, so it waits.
    if (receivers(SIGNAL(linkRequested(QString))) == 0) {
        m_pendingLink = link;
        return true;
    }
    emit linkRequested(link);
    return true;
}

QString AppService::takePendingLink()
{
    const QString link = m_pendingLink;
    m_pendingLink.clear();
    return link;
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

void AppServiceAdaptor::openUrl(const QString &link)
{
    // The window comes forward either way: the user tapped a link and expects
    // to see this app, whether or not it can do anything with it.
    m_service->requestRaise();
    m_service->requestLink(link);
}
