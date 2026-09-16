#ifndef IMAGEFACTS_H
#define IMAGEFACTS_H

#include <QtGlobal>
#include <QString>
#include <QStringList>

/// What a picture file really is, read from its header. `frames` is 1 for a
/// still, `width`/`height` are the canvas a frame is composed onto.
struct ImageFacts {
    QString format;
    int width = 0;
    int height = 0;
    int frames = 1;
    /// What a decoder would have to allocate, from the file's own header. For
    /// GIF that is the logical screen, which the frame size does not give away.
    qint64 canvasPixels = 0;
};

/// Reads the header, never the pixels. An unreadable file answers a still of
/// size zero — the answer that offers nothing.
ImageFacts imageFacts(const QString &path);

/// The picture formats this device can actually animate, as the subtype of
/// their media type ("gif"). Asked of Qt, not prescribed: the plugin set
/// decides, and on this Qt the WebP handler answers only size and quality.
QStringList animatableFormats();

/// Whether this file may be played. `AnimatedImage` has no writable
/// `sourceSize`, so nothing bounds its decode: the ceiling has to hold here,
/// before the file is handed over.
bool mayAnimate(const ImageFacts &facts);

/// Whether a decoder would have to hold more pixels than this device can spare.
/// Read from the header of the file that actually arrived: the event's declared
/// measurements are the sender's word, they may be missing altogether, and
/// `sourceSize` is a hint some formats ignore.
bool imageBeyondDecodeBudget(const QString &path);

#endif // IMAGEFACTS_H
