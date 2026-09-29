#include "camerashots.h"

#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QUrl>

static QString localPath(const QString &path)
{
    return path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path;
}

CameraShots::CameraShots(const QString &directory, QObject *parent)
    : QObject(parent)
    , m_directory(QDir::cleanPath(directory))
{
    QDir dir(m_directory);
    if (!m_directory.isEmpty() && dir.exists()) {
        for (const QString &name : dir.entryList(QDir::Files)) {
            dir.remove(name);
        }
    }
}

QString CameraShots::newPath()
{
    if (m_directory.isEmpty() || !QDir().mkpath(m_directory)) {
        return QString();
    }
    const QString stem = m_directory + QStringLiteral("/photo-")
            + QDateTime::currentDateTime().toString(QStringLiteral("yyyyMMdd-hhmmss"));
    QString candidate = stem + QStringLiteral(".jpg");
    for (int n = 1; QFile::exists(candidate); ++n) {
        candidate = stem + QLatin1Char('-') + QString::number(n) + QStringLiteral(".jpg");
    }
    return candidate;
}

void CameraShots::discard(const QString &path)
{
    const QString local = localPath(path);
    if (owns(local)) {
        QFile::remove(local);
    }
}

bool CameraShots::owns(const QString &path) const
{
    if (m_directory.isEmpty() || path.isEmpty()) {
        return false;
    }
    return QFileInfo(localPath(path)).absolutePath() == m_directory;
}
