#include "videostill.h"

#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QImageReader>
#include <QStandardPaths>

// A video's preview is not made here. The platform ships `thumbnaild-video`
// for it - a process of its own on ffmpeg, the same one the gallery uses, and
// it reads the rotation a phone writes into the file. Decoding a stranger's
// video inside this process was measured and cost the app: an own pipeline
// parked a thread in a futex and the message never went out. A process can be
// taken away; a decoder in this one cannot.

namespace {

/// The longest edge the still keeps - what a chat bubble shows, and what a
/// receiver's own thumbnail request would have asked the server for anyway.
const int StillEdge = 800;
/// Stills older than this go on the next send.
const qint64 KeepSeconds = 24 * 60 * 60;

QString stillDirectory()
{
    const QString base = QStandardPaths::writableLocation(QStandardPaths::CacheLocation);
    if (base.isEmpty()) {
        return QString();
    }
    QDir directory(base);
    if (!directory.exists(QStringLiteral("stills"))
            && !directory.mkpath(QStringLiteral("stills"))) {
        return QString();
    }
    return directory.absoluteFilePath(QStringLiteral("stills"));
}

/// Named after what went into it, so sending the same file twice reuses the
/// frame and two different files cannot collide.
QString stillName(const QFileInfo &info)
{
    QCryptographicHash hash(QCryptographicHash::Sha256);
    hash.addData(info.absoluteFilePath().toUtf8());
    hash.addData(QByteArray::number(info.size()));
    hash.addData(QByteArray::number(info.lastModified().toMSecsSinceEpoch()));
    return QString::fromLatin1(hash.result().toHex().left(32)) + QStringLiteral(".jpg");
}

} // namespace

QString videoStillPath(const QString &path)
{
    const QFileInfo info(path);
    const QString directory = stillDirectory();
    if (!info.isFile() || directory.isEmpty()) {
        return QString();
    }
    return QDir(directory).absoluteFilePath(stillName(info));
}

QString videoStillProgram()
{
    return QStringLiteral("thumbnaild-video");
}

QStringList videoStillArguments(const QString &path, const QString &target)
{
    // No `--crop`: a cropped preview promises a frame the film does not have.
    return QStringList()
            << QStringLiteral("--output") << target
            << QStringLiteral("--width") << QString::number(StillEdge)
            << QStringLiteral("--height") << QString::number(StillEdge)
            << path;
}

VideoStill readVideoStill(const QString &target)
{
    VideoStill still;
    if (!QFile::exists(target)) {
        return still;
    }
    // The header, never the pixels: the helper decided the size, this reads
    // back what it wrote so the event can declare it.
    QImageReader reader(target);
    const QSize size = reader.size();
    if (!size.isValid() || size.isEmpty()) {
        return still;
    }
    still.path = target;
    still.width = size.width();
    still.height = size.height();
    return still;
}

void pruneVideoStills()
{
    const QString directory = stillDirectory();
    if (directory.isEmpty()) {
        return;
    }
    const QDateTime cutoff = QDateTime::currentDateTime().addSecs(-KeepSeconds);
    const QFileInfoList entries = QDir(directory).entryInfoList(QDir::Files);
    for (const QFileInfo &entry : entries) {
        if (entry.lastModified() < cutoff) {
            QFile::remove(entry.absoluteFilePath());
        }
    }
}
