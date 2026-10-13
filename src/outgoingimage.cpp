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

#include <cstring>

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

/// Skips GIF data sub-blocks up to their terminator; false on a short file.
bool skipSubBlocks(QFile &file)
{
    for (;;) {
        char size = 0;
        if (!file.getChar(&size)) {
            return false;
        }
        if (size == 0) {
            return true;
        }
        if (!file.seek(file.pos() + quint8(size))) {
            return false;
        }
    }
}

/// A GIF's comment and XMP extensions, walked by block structure: the two
/// marker bytes alone also occur inside the compressed pixels.
bool gifCarriesMetadata(QFile &file)
{
    QByteArray screen;
    if (!file.seek(0) || (screen = file.read(13)).size() < 13) {
        return true;
    }
    const quint8 packed = quint8(screen.at(10));
    if (packed & 0x80) {
        file.seek(13 + 3 * (1 << ((packed & 0x07) + 1)));
    }
    for (;;) {
        char kind = 0;
        if (!file.getChar(&kind)) {
            return true;
        }
        if (quint8(kind) == 0x3b) {
            return false;
        }
        if (quint8(kind) == 0x21) {
            char label = 0;
            if (!file.getChar(&label)) {
                return true;
            }
            if (quint8(label) == 0xfe) {
                return true;
            }
            if (quint8(label) == 0xff) {
                const QByteArray application = file.peek(12);
                if (application.size() < 12 || application.mid(1, 8) == "XMP Data") {
                    return true;
                }
            }
            if (!skipSubBlocks(file)) {
                return true;
            }
            continue;
        }
        if (quint8(kind) != 0x2c) {
            return true;
        }
        const QByteArray descriptor = file.read(9);
        if (descriptor.size() < 9) {
            return true;
        }
        const quint8 local = quint8(descriptor.at(8));
        if (local & 0x80) {
            file.seek(file.pos() + 3 * (1 << ((local & 0x07) + 1)));
        }
        char codeSize = 0;
        if (!file.getChar(&codeSize) || !skipSubBlocks(file)) {
            return true;
        }
    }
}

/// PNG text and EXIF chunks, by chunk type; the scan below knows only XMP.
bool pngCarriesMetadata(QFile &file)
{
    if (!file.seek(8)) {
        return true;
    }
    for (;;) {
        const QByteArray header = file.read(8);
        if (header.size() < 8) {
            return true;
        }
        const QByteArray type = header.mid(4, 4);
        if (type == "tEXt" || type == "zTXt" || type == "iTXt" || type == "eXIf" || type == "tIME") {
            return true;
        }
        if (type == "IEND") {
            return false;
        }
        const quint32 length = (quint32(quint8(header.at(0))) << 24) | (quint32(quint8(header.at(1))) << 16)
                | (quint32(quint8(header.at(2))) << 8) | quint32(quint8(header.at(3)));
        if (!file.seek(file.pos() + qint64(length) + 4)) {
            return true;
        }
    }
}

/// EXIF or XMP anywhere in the file: PNG may put its chunk after the pixels,
/// and WebP names its chunks without the JPEG marker.
bool carriesMetadata(const QString &path)
{
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        // Unknown counts as carrying.
        return true;
    }
    const QByteArray head = file.peek(12);
    // TIFF and DNG keep EXIF in IFDs, without a marker to scan for.
    if (head.startsWith(QByteArray("II*\0", 4)) || head.startsWith(QByteArray("MM\0*", 4))) {
        return true;
    }
    if (head.startsWith("GIF87a") || head.startsWith("GIF89a")) {
        if (gifCarriesMetadata(file)) {
            return true;
        }
    } else if (head.startsWith(QByteArray("\x89PNG\r\n\x1a\n", 8))) {
        if (pngCarriesMetadata(file)) {
            return true;
        }
    }
    if (!file.seek(0)) {
        return true;
    }
    const bool webp = head.startsWith("RIFF") && head.mid(8, 4) == "WEBP";
    const QList<QByteArray> marks = {
        QByteArrayLiteral("Exif\0\0"), QByteArrayLiteral("http://ns.adobe.com/xap/"),
        QByteArrayLiteral("eXIf"), QByteArrayLiteral("iTXtXML:com.adobe.xmp")
    };
    const int overlap = 32;
    QByteArray carry;
    while (!file.atEnd()) {
        const QByteArray chunk = carry + file.read(1024 * 1024);
        if (chunk.isEmpty()) {
            break;
        }
        for (const QByteArray &mark : marks) {
            if (chunk.contains(mark)) {
                return true;
            }
        }
        if (webp && (chunk.contains("EXIF") || chunk.contains("XMP "))) {
            return true;
        }
        carry = chunk.right(overlap);
    }
    return file.error() != QFileDevice::NoError;
}

/// A drawing goes as it is, so it is read as text: editor and RDF blocks name
/// people and paths. Compressed, it cannot be read and counts as carrying.
bool svgCarriesMetadata(const QString &path)
{
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly) || file.size() > 16 * 1024 * 1024) {
        return true;
    }
    const QByteArray raw = file.readAll();
    // Compressed, or not UTF-8 (UTF-16 hides every mark below): not read, not sent.
    if (raw.startsWith("\x1f\x8b") || raw.startsWith("\xff\xfe") || raw.startsWith("\xfe\xff")) {
        return true;
    }
    const QByteArray text = raw.toLower();
    // Embedded pictures carry their own metadata; titles, descriptions and
    // links name things the drawing does not show.
    const QList<QByteArray> marks = {
        QByteArrayLiteral("<metadata"), QByteArrayLiteral("inkscape:"), QByteArrayLiteral("sodipodi:"),
        QByteArrayLiteral("rdf:rdf"), QByteArrayLiteral("http://ns.adobe.com/xap/"),
        QByteArrayLiteral("data:image"), QByteArrayLiteral("<title"), QByteArrayLiteral("<desc"),
        QByteArrayLiteral("file:/")
    };
    for (const QByteArray &mark : marks) {
        if (text.contains(mark)) {
            return true;
        }
    }
    return file.error() != QFileDevice::NoError;
}

/// The original, where the copy could not be made - unless it would carry
/// what the copy was meant to leave behind.
OutgoingImage originalIfClean(const QString &path, const QString &mimeType)
{
    OutgoingImage out = { path, mimeType };
    if (carriesMetadata(path)) {
        qWarning("xmatic: a picture with metadata could not be re-encoded and is not sent");
        out.refused = true;
    }
    return out;
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

/// The pixels alone: `QImage::text()` holds the source's PNG text chunks and
/// every copy, scale and save carries them on.
QImage withoutText(const QImage &image)
{
    QImage fresh(image.size(), image.format());
    if (fresh.isNull()) {
        return QImage();
    }
    fresh.setColorTable(image.colorTable());
    const int bytes = qMin(fresh.bytesPerLine(), image.bytesPerLine());
    for (int y = 0; y < image.height(); ++y) {
        memcpy(fresh.scanLine(y), image.constScanLine(y), size_t(bytes));
    }
    return fresh;
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
        return withoutText(source);
    }
    return withoutText(source.width() >= source.height()
                       ? source.scaledToWidth(edge, Qt::SmoothTransformation)
                       : source.scaledToHeight(edge, Qt::SmoothTransformation));
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
    // loses every frame but one, the second the point of being a drawing.
    if (mimeType == QLatin1String("image/gif")) {
        return originalIfClean(path, mimeType);
    }
    if (mimeType.contains(QLatin1String("svg"))) {
        OutgoingImage out = untouched;
        if (svgCarriesMetadata(path)) {
            qWarning("xmatic: a drawing with editor metadata is not sent");
            out.refused = true;
        }
        return out;
    }

    const QFileInfo info(path);
    if (!info.exists() || !info.isReadable()) {
        return untouched;
    }

    const bool screenshot = looksLikeScreenshot(path);
    const QImage image = boundedImage(path, screenshot ? ShotEdge : PhotoEdge);
    // A format this Qt cannot decode, or a damaged file.
    if (image.isNull()) {
        return originalIfClean(path, mimeType);
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
            return originalIfClean(path, mimeType);
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
        return originalIfClean(path, mimeType);
    }

    // Not smaller: the original goes, unless it carries metadata.
    if (encoded.size() >= info.size() && !carriesMetadata(path)) {
        return untouched;
    }

    const QString directory = outgoingDirectory();
    if (directory.isEmpty()) {
        return originalIfClean(path, mimeType);
    }
    const QString target = directory + QLatin1Char('/') + copyName(info, screenshot, suffix);

    QFile file(target);
    if (!file.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        return originalIfClean(path, mimeType);
    }
    if (file.write(encoded) != encoded.size() || !file.flush()) {
        file.close();
        file.remove();
        return originalIfClean(path, mimeType);
    }
    file.close();
    // Checked on what goes out, not on what went in.
    if (carriesMetadata(target)) {
        QFile::remove(target);
        qWarning("xmatic: a re-encoded picture still carried metadata and is not sent");
        OutgoingImage out = { path, mimeType };
        out.refused = true;
        return out;
    }

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
