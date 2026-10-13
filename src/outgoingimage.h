#ifndef OUTGOINGIMAGE_H
#define OUTGOINGIMAGE_H

#include <QString>

/// What actually goes out for a picked picture: either the file itself or a
/// re-encoded copy in the cache. The mime type travels with it because a
/// re-encode may change it.
struct OutgoingImage {
    QString path;
    QString mimeType;
    /// Could not be cleaned and carries metadata: nothing is sent. Left out of
    /// a `{ path, mime }` initialiser it is false.
    bool refused;
};

/// Shrinks `path` towards `TargetBytes` unless `original` says otherwise.
/// Anything that is not an image, is already small enough, or would not get
/// smaller comes back unchanged - unless it carries metadata. A picture that
/// cannot be re-encoded and does carry some comes back `refused`. GIF and SVG
/// go as they are, or are refused the same way. The copy is in the cache.
OutgoingImage prepareOutgoingImage(const QString &path, const QString &mimeType,
                                   bool original);

/// Drops re-encoded copies older than a day. The upload runs asynchronously,
/// so a copy cannot be deleted right after handing it over.
void pruneOutgoingImages();

#endif // OUTGOINGIMAGE_H
