#include "imagefacts.h"

#include <QFile>
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

/// A still is held whole while it is scaled, four bytes to the pixel. Forty
/// megapixels is 160 MB for one picture - past any camera this platform meets
/// and well short of what a crafted header can ask for.
const qint64 MaxStillPixels = 40 * 1000 * 1000;

} // namespace

namespace {

quint32 beU32(const uchar *at) { return (quint32(at[0]) << 24) | (quint32(at[1]) << 16) | (quint32(at[2]) << 8) | at[3]; }
quint16 beU16(const uchar *at) { return quint16((quint16(at[0]) << 8) | at[1]); }
quint16 leU16(const uchar *at) { return quint16((quint16(at[1]) << 8) | at[0]); }

/// The canvas a decoder would have to allocate, read from the file's own header
/// and nothing else. Deliberately *not* `QImageReader::size()`:
///
///  * for GIF that answers the size of the first **frame**, while the decoder
///    allocates the logical screen - measured: a 1x1 frame on a 10000x10000
///    canvas reads back as one pixel and then takes 410 MB;
///  * reading a PNG header through libpng inflates every `zTXt` chunk on the
///    way - measured: a 771 kB file, 6.9 s and 3.8 GB;
///  * a JPEG header read keeps every COM and APP1 segment - measured: 530 MB.
///
/// This walks bytes, allocates nothing, and knows only the four formats that
/// matter here. Anything else answers zero, which means "not this function's
/// business" - a document or a video never reaches a decoder because of it.
qint64 canvasPixels(const QString &path, bool *decodesScaled = nullptr)
{
    if (decodesScaled) {
        *decodesScaled = false;
    }
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        return 0;
    }

    const QByteArray head = file.read(32);
    const uchar *bytes = reinterpret_cast<const uchar *>(head.constData());

    // PNG: signature, then IHDR - length, type, width, height.
    static const char PngSignature[] = "\x89PNG\r\n\x1a\n";
    if (head.size() >= 24 && head.startsWith(QByteArray(PngSignature, 8))
        && head.mid(12, 4) == QByteArray("IHDR", 4)) {
        return qint64(beU32(bytes + 16)) * qint64(beU32(bytes + 20));
    }

    // GIF: the logical screen, which is what the decoder allocates.
    if (head.size() >= 10
        && (head.startsWith(QByteArray("GIF87a", 6)) || head.startsWith(QByteArray("GIF89a", 6)))) {
        return qint64(leU16(bytes + 6)) * qint64(leU16(bytes + 8));
    }

    // WebP: VP8X carries a 24-bit canvas, VP8 a 14-bit one, VP8L packs both.
    if (head.size() >= 30 && head.startsWith(QByteArray("RIFF", 4))
        && head.mid(8, 4) == QByteArray("WEBP", 4)) {
        const QByteArray kind = head.mid(12, 4);
        if (kind == QByteArray("VP8X", 4)) {
            // Canvas width and height minus one, three bytes each, little endian.
            const qint64 width = 1 + (qint64(bytes[24]) | (qint64(bytes[25]) << 8)
                                      | (qint64(bytes[26]) << 16));
            const qint64 height = 1 + (qint64(bytes[27]) | (qint64(bytes[28]) << 8)
                                       | (qint64(bytes[29]) << 16));
            return width * height;
        }
        if (kind == QByteArray("VP8 ", 4) && head.size() >= 30) {
            return qint64(leU16(bytes + 26) & 0x3fff) * qint64(leU16(bytes + 28) & 0x3fff);
        }
        if (kind == QByteArray("VP8L", 4) && head.size() >= 25) {
            const quint32 packed = quint32(bytes[21]) | (quint32(bytes[22]) << 8)
                    | (quint32(bytes[23]) << 16) | (quint32(bytes[24]) << 24);
            return qint64((packed & 0x3fff) + 1) * qint64(((packed >> 14) & 0x3fff) + 1);
        }
        return 0;
    }

    // JPEG: walk the markers to the frame header, skipping every segment by its
    // declared length rather than reading it.
    if (head.size() >= 2 && bytes[0] == 0xff && bytes[1] == 0xd8) {
        // libjpeg decodes to a fraction of the full size when a target size is
        // given, and both places that draw a picture give one. A ceiling on the
        // stored measurements would only refuse honest photographs.
        if (decodesScaled) {
            *decodesScaled = true;
        }
        file.seek(2);
        // Bounded: a header this far in is a file playing for time.
        const qint64 Ceiling = 4 * 1024 * 1024;
        while (file.pos() < Ceiling) {
            QByteArray marker = file.read(2);
            if (marker.size() < 2) {
                return 0;
            }
            const uchar *at = reinterpret_cast<const uchar *>(marker.constData());
            if (at[0] != 0xff) {
                return 0;
            }
            const uchar kind = at[1];
            // Standalone markers carry no length.
            if (kind == 0xd8 || kind == 0x01 || (kind >= 0xd0 && kind <= 0xd7)) {
                continue;
            }
            const QByteArray sizeBytes = file.read(2);
            if (sizeBytes.size() < 2) {
                return 0;
            }
            const int length = beU16(reinterpret_cast<const uchar *>(sizeBytes.constData()));
            if (length < 2) {
                return 0;
            }
            // SOF0..SOF15, less the four that are not frame headers.
            const bool frame = kind >= 0xc0 && kind <= 0xcf
                    && kind != 0xc4 && kind != 0xc8 && kind != 0xcc;
            if (frame) {
                const QByteArray frameHeader = file.read(5);
                if (frameHeader.size() < 5) {
                    return 0;
                }
                const uchar *shape = reinterpret_cast<const uchar *>(frameHeader.constData());
                return qint64(beU16(shape + 1)) * qint64(beU16(shape + 3));
            }
            file.seek(file.pos() + length - 2);
        }
        return 0;
    }

    return 0;
}

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

    // The canvas from the file's own header, where this can read it: for GIF the
    // handler answers the size of the first *frame*, and a one-pixel frame on a
    // ten-thousand-pixel canvas passed every ceiling in this file.
    const qint64 canvas = canvasPixels(local);
    if (canvas > 0 && canvas > qint64(facts.width) * qint64(facts.height)) {
        facts.canvasPixels = canvas;
    } else {
        facts.canvasPixels = qint64(facts.width) * qint64(facts.height);
    }

    // Only where the plugin says it animates: `imageCount` is one on every
    // other handler, and asking it there would still scan the file.
    if (reader.supportsAnimation()) {
        facts.frames = qMax(1, reader.imageCount());
    }

    return facts;
}


bool imageBeyondDecodeBudget(const QString &path)
{
    QString local = path;
    if (local.startsWith(QLatin1String("file://"))) {
        local = QUrl(local).toLocalFile();
    }

    bool decodesScaled = false;
    const qint64 pixels = canvasPixels(local, &decodesScaled);
    if (decodesScaled) {
        // A fifty-megapixel photograph is ordinary and costs this device nothing
        // it is not already paying for a small one.
        return false;
    }
    if (pixels <= 0) {
        // Not one of the formats this knows, or a header that answers nothing.
        // Refusing here would hide ordinary files; the ceiling is a decoder
        // question and this is the only place that can answer it cheaply.
        return false;
    }
    return pixels > MaxStillPixels;
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
    // The canvas, not the first frame: a GIF whose first frame is one pixel
    // still has its decoder allocate the whole screen, every frame.
    const qint64 pixels = qMax(facts.canvasPixels, qint64(facts.width) * qint64(facts.height));
    return pixels <= MaxAnimatedPixels;
}
