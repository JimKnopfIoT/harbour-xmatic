// xmatic — native Matrix client for Sailfish OS.
//
// The Qt side is deliberately thin: it owns the Silica UI and the list models,
// while all Matrix protocol work (sync, timeline, encryption) lives in the Rust
// core behind the C ABI in src/generated/xmatic_core.h.

#include <QDir>
#include <QGuiApplication>
#include <QQmlContext>
#include <QQuickView>
#include <QScopedPointer>
#include <QStandardPaths>
#include <QString>

#include <sailfishapp.h>

#include "matrixbridge.h"

// Set by the spec file at package build time as bare tokens (quotes do not
// survive the rpm/shell/qmake chain); stringified here. A plain qmake build
// gets "dev".
#ifndef XMATIC_VERSION
#define XMATIC_VERSION dev
#endif
#define XMATIC_STRINGIFY_1(x) #x
#define XMATIC_STRINGIFY(x) XMATIC_STRINGIFY_1(x)

namespace {

// Everything the core persists — state store, crypto store, session — lives
// here. Under Sailjail this resolves below the organisation and application
// name declared in the desktop file, and the sandbox grants it without any
// extra permission.
QString ensureDirectory(QStandardPaths::StandardLocation location)
{
    const QString path = QStandardPaths::writableLocation(location);
    if (path.isEmpty()) {
        return QString();
    }
    QDir().mkpath(path);
    return path;
}

} // namespace

int main(int argc, char *argv[])
{
    QScopedPointer<QGuiApplication> app(SailfishApp::application(argc, argv));

    const QString dataDirectory = ensureDirectory(QStandardPaths::AppDataLocation);
    if (dataDirectory.isEmpty()) {
        qWarning("xmatic: no writable data directory; the core cannot start");
    }
    // Attachments go to the cache: they can always be downloaded again, and
    // the system may reclaim the space.
    const QString cacheDirectory = ensureDirectory(QStandardPaths::CacheLocation);

    MatrixBridge bridge(dataDirectory, cacheDirectory);
    qInfo("xmatic: %s", qPrintable(bridge.coreVersion()));

    QScopedPointer<QQuickView> view(SailfishApp::createView());
    view->rootContext()->setContextProperty(QStringLiteral("matrix"), &bridge);
    view->rootContext()->setContextProperty(QStringLiteral("appVersion"),
                                            QStringLiteral(XMATIC_STRINGIFY(XMATIC_VERSION)));
    view->setSource(SailfishApp::pathTo(QStringLiteral("qml/harbour-xmatic.qml")));
    view->show();

    return app->exec();
}
