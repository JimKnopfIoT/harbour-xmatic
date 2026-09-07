#ifndef IMAGEFACTS_H
#define IMAGEFACTS_H

#include <QString>
#include <QStringList>

/// What a picture file really is, read from its header. `frames` is 1 for a
/// still, `width`/`height` are the canvas a frame is composed onto.
struct ImageFacts {
    QString format;
    int width = 0;
    int height = 0;
    int frames = 1;
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

#endif // IMAGEFACTS_H
