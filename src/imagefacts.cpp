#include "imagefacts.h"

#include <QFileInfo>
#include <QImageReader>
#include <QMovie>
#include <QSize>
#include <QUrl>

namespace {

/// One frame, in pixels. A frame is held at its full size and this is the only
/// thing standing in front of that allocation.
const qint64 MaxAnimatedPixels = 2048 * 2048;

/// Reading the frame count walks the whole file, so this bounds a read on the
/// UI thread as much as it bounds the file. Past it, whatever it is, it is not
/// something to play on this hardware.
const qint64 MaxScanBytes = 16 * 1024 * 1024;

} // namespace

ImageFacts imageFacts(const QString &path)
{
    QString local = path;
    if (local.startsWith(QLatin1String("file://"))) {
        local = QUrl(local).toLocalFile();
    }

    ImageFacts facts;

    const QFileInfo info(local);
    if (!info.isFile() || info.size() > MaxScanBytes) {
        return facts;
    }

    QImageReader reader(local);
    // The name of a file out of a room says nothing about its content.
    reader.setDecideFormatFromContent(true);
    if (!reader.canRead()) {
        return facts;
    }

    facts.format = QString::fromLatin1(reader.format());

    const QSize size = reader.size();
    if (size.isValid()) {
        facts.width = size.width();
        facts.height = size.height();
    }

    // Only where the plugin says it animates: `imageCount` is one on every
    // other handler, and asking it there would still scan the file.
    if (reader.supportsAnimation()) {
        facts.frames = qMax(1, reader.imageCount());
    }

    return facts;
}

QStringList animatableFormats()
{
    QStringList formats;
    // The formats whose handler answers the animation question at all - which is
    // a smaller set than the ones that can be read.
    foreach (const QByteArray &format, QMovie::supportedFormats()) {
        formats.append(QString::fromLatin1(format).toLower());
    }
    return formats;
}

bool mayAnimate(const ImageFacts &facts)
{
    if (facts.frames <= 1 || facts.width <= 0 || facts.height <= 0) {
        return false;
    }
    return qint64(facts.width) * qint64(facts.height) <= MaxAnimatedPixels;
}
