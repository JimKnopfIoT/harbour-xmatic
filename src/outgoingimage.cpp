#include "outgoingimage.h"

#include <QBuffer>
#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QImage>
#include <QImageReader>
#include <QStandardPaths>

namespace {

/// What a picture may weigh before it is re-encoded.
const qint64 TargetBytes = 600 * 1024;
/// The longest edge a photo keeps. Above this nothing is gained that a phone
/// screen can show.
const int PhotoEdge = 1920;
/// A screenshot keeps every pixel it has: it *is* the screen, and scaling it
/// is what makes it unreadable. Measured on the device - a portrait shot taken
/// to 491x1080 was both illegible and still 688 KiB, because smooth scaling
/// turns flat colour into gradients that PNG cannot pack.
const int ShotEdge = 4096;
/// Downwards from here, never below the last one: past that the picture is
/// worse than the bytes are worth.
const int Qualities[] = { 85, 75, 65, 55, 45 };

/// The cache directory the copies live in, created on demand.
QString outgoingDirectory()
{
    const QString base = QStandardPaths::writableLocation(QStandardPaths::CacheLocation);
    if (base.isEmpty()) {
        return QString();
    }
    QDir directory(base);
    if (!directory.exists(QStringLiteral("outgoing"))
            && !directory.mkpath(QStringLiteral("outgoing"))) {
        return QString();
    }
    return directory.absoluteFilePath(QStringLiteral("outgoing"));
}

/// Sailfish puts them in their own folder, and that is the only honest signal:
/// a screenshot carries nothing in the file that says what it is.
bool looksLikeScreenshot(const QString &path)
{
    return path.contains(QStringLiteral("/Screenshots/"), Qt::CaseInsensitive);
}

/// Names the copy after what went into it, so a second send of the same file
/// reuses it and two different files cannot collide.
QString copyName(const QFileInfo &info, bool screenshot, const QString &suffix)
{
    QCryptographicHash hash(QCryptographicHash::Sha256);
    hash.addData(info.absoluteFilePath().toUtf8());
    hash.addData(QByteArray::number(info.size()));
    hash.addData(QByteArray::number(info.lastModified().toMSecsSinceEpoch()));
    hash.addData(screenshot ? "shot" : "photo");
    return QString::fromLatin1(hash.result().toHex().left(32)) + QLatin1Char('.') + suffix;
}

QImage boundedImage(const QString &path, int edge)
{
    QImageReader reader(path);
    // The orientation lives in the metadata that the re-encode drops, so it has
    // to be baked into the pixels here or the picture goes out lying on its side.
    reader.setAutoTransform(true);
    const QImage source = reader.read();
    if (source.isNull()) {
        return QImage();
    }
    if (source.width() <= edge && source.height() <= edge) {
        return source;
    }
    return source.width() >= source.height()
            ? source.scaledToWidth(edge, Qt::SmoothTransformation)
            : source.scaledToHeight(edge, Qt::SmoothTransformation);
}

} // namespace

OutgoingImage prepareOutgoingImage(const QString &path, const QString &mimeType,
                                   bool original)
{
    const OutgoingImage untouched = { path, mimeType };

    if (original || !mimeType.startsWith(QLatin1String("image/"))) {
        return untouched;
    }
    // An animation and a drawing are not photographs: re-encoding the first
    // loses every frame but one, the second loses the point of being a drawing.
    if (mimeType == QLatin1String("image/gif") || mimeType.contains(QLatin1String("svg"))) {
        return untouched;
    }

    const QFileInfo info(path);
    if (!info.exists() || !info.isReadable()) {
        return untouched;
    }

    const bool screenshot = looksLikeScreenshot(path);
    const QImage image = boundedImage(path, screenshot ? ShotEdge : PhotoEdge);
    if (image.isNull()) {
        return untouched;
    }

    QByteArray encoded;
    QString suffix;
    QString outgoingMime;

    // Transparency has no choice: JPEG turns it black, so those keep the PNG
    // whatever it weighs. A screenshot only tries it - flat colour packs small
    // and every pixel of the text survives.
    const bool transparent = image.hasAlphaChannel();
    if (screenshot || transparent) {
        QBuffer buffer(&encoded);
        buffer.open(QIODevice::WriteOnly);
        if (!image.save(&buffer, "PNG")) {
            return untouched;
        }
        suffix = QStringLiteral("png");
        outgoingMime = QStringLiteral("image/png");
    }

    if (!transparent && (encoded.isEmpty() || encoded.size() > TargetBytes)) {
        // Quality first, size second: the loop stops at the first setting that
        // fits, and keeps the last one where nothing does.
        QByteArray jpeg;
        for (unsigned int i = 0; i < sizeof(Qualities) / sizeof(Qualities[0]); ++i) {
            QByteArray attempt;
            QBuffer buffer(&attempt);
            buffer.open(QIODevice::WriteOnly);
            if (!image.save(&buffer, "JPEG", Qualities[i])) {
                break;
            }
            jpeg = attempt;
            if (attempt.size() <= TargetBytes) {
                break;
            }
        }
        // A screen busy enough to defeat PNG goes as a photograph: sharpness is
        // worth less than arriving. Only ever the smaller of the two, though.
        if (!jpeg.isEmpty() && (encoded.isEmpty() || jpeg.size() < encoded.size())) {
            encoded = jpeg;
            suffix = QStringLiteral("jpg");
            outgoingMime = QStringLiteral("image/jpeg");
        }
    }

    if (encoded.isEmpty()) {
        return untouched;
    }

    // A copy that is not smaller has cost the user quality for nothing.
    if (encoded.size() >= info.size()) {
        return untouched;
    }

    const QString directory = outgoingDirectory();
    if (directory.isEmpty()) {
        return untouched;
    }
    const QString target = directory + QLatin1Char('/') + copyName(info, screenshot, suffix);

    QFile file(target);
    if (!file.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        return untouched;
    }
    if (file.write(encoded) != encoded.size() || !file.flush()) {
        file.close();
        file.remove();
        return untouched;
    }
    file.close();

    // The route belongs in the line: without it, a size alone does not say
    // whether the picture was scaled, re-encoded, or both.
    qInfo("xmatic: attachment re-encoded to %dx%d %s, %lld KiB to %lld KiB",
          image.width(), image.height(), qPrintable(outgoingMime),
          static_cast<long long>(info.size() / 1024),
          static_cast<long long>(encoded.size() / 1024));

    OutgoingImage out = { target, outgoingMime };
    return out;
}

void pruneOutgoingImages()
{
    const QString directory = outgoingDirectory();
    if (directory.isEmpty()) {
        return;
    }
    const QDateTime cutoff = QDateTime::currentDateTime().addDays(-1);
    const QFileInfoList entries = QDir(directory).entryInfoList(QDir::Files);
    for (int i = 0; i < entries.size(); ++i) {
        if (entries.at(i).lastModified() < cutoff) {
            QFile::remove(entries.at(i).absoluteFilePath());
        }
    }
}
