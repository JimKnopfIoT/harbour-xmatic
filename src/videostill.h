#ifndef VIDEOSTILL_H
#define VIDEOSTILL_H

#include <QString>
#include <QStringList>

/// One frame out of a video, as a JPEG in the cache. Empty `path` means none
/// could be had - the video then goes out without a preview, as it did before.
struct VideoStill {
    QString path;
    int width = 0;
    int height = 0;
};

/// Where the still for `path` belongs. Named after the file's own facts, so
/// sending the same video twice reuses the frame.
QString videoStillPath(const QString &path);

/// The platform's own video thumbnailer and how it is called. It is a separate
/// process on purpose: a decoder that does not come back can be killed, and one
/// measured on the device did not come back.
QString videoStillProgram();
QStringList videoStillArguments(const QString &path, const QString &target);

/// What the helper wrote, measured from the file's header.
VideoStill readVideoStill(const QString &target);

/// Drops stills older than a day. The upload runs asynchronously, so the file
/// cannot be deleted right after it is handed over.
void pruneVideoStills();

#endif // VIDEOSTILL_H
