#ifndef CAMERASOURCE_H
#define CAMERASOURCE_H

#include <QAbstractVideoSurface>
#include <QByteArray>
#include <QSize>
#include <QString>

class QCamera;

/// Captures the camera through Qt rather than GStreamer.
///
/// Sailfish's `droidcamsrc` comes from the Android adaptation layer: it has
/// several outputs, needs a mode and an explicit start before video flows, and
/// hands out hardware buffers. Driven like an ordinary source it delivers
/// exactly one frame — which is what this class exists to avoid. Qt's camera
/// stack already wraps all of that, so frames are taken from there and pushed
/// into the pipeline.
class CameraSource : public QAbstractVideoSurface
{
    Q_OBJECT

public:
    explicit CameraSource(QObject *parent = nullptr);
    ~CameraSource() override;

    QList<QVideoFrame::PixelFormat> supportedPixelFormats(
        QAbstractVideoBuffer::HandleType type = QAbstractVideoBuffer::NoHandle) const override;

    bool present(const QVideoFrame &frame) override;

    /// Whether a camera can be opened at all. Sailfish does not always
    /// enumerate its cameras, so the default device counts as available.
    static bool isAvailable();

    /// Opens the front camera and starts delivering frames.
    void start();
    void stop();

signals:
    /// One captured frame, already in a layout GStreamer understands.
    /// `format` is a GStreamer video format name such as "BGRx" or "NV21".
    void frameReady(const QByteArray &data, int width, int height, const QString &format);

    void failed(const QString &message);

private:
    QCamera *m_camera = nullptr;
};

#endif // CAMERASOURCE_H
