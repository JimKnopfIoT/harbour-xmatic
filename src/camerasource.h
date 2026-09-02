#ifndef CAMERASOURCE_H
#define CAMERASOURCE_H

#include <QAbstractVideoSurface>
#include <QByteArray>
#include <QSize>
#include <QString>

class QCamera;

/// Captures through Qt rather than GStreamer: `droidcamsrc` needs a mode and an
/// explicit start and otherwise delivers exactly one frame.
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
