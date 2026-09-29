#ifndef CAMERASHOTS_H
#define CAMERASHOTS_H

#include <QObject>
#include <QString>

/// Photos taken in the picker, as QML reads them: `matrix.cameraShots`. They
/// live in the cache and are throwaway: gone once sent, discarded or left over.
/// Keeping one in the gallery is a copy through `saveToPictures`.
class CameraShots : public QObject
{
    Q_OBJECT

public:
    /// Clears what an earlier run left behind.
    explicit CameraShots(const QString &directory, QObject *parent = nullptr);

    /// Where the next photo goes; empty when the directory cannot be made.
    Q_INVOKABLE QString newPath();
    /// Deletes a photo of ours; anything outside the directory is left alone.
    Q_INVOKABLE void discard(const QString &path);

    bool owns(const QString &path) const;

private:
    QString m_directory;
};

#endif // CAMERASHOTS_H
