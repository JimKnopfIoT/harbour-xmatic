#ifndef VIDEOSTREAM_H
#define VIDEOSTREAM_H

#include <QAbstractVideoSurface>
#include <QImage>
#include <QObject>
#include <QSize>
#include <QVideoFrame>

/// Carries decoded frames into QML. The device has no GStreamer sink that draws
/// into a QML scene, so frames are pulled out as images - one conversion each.
class VideoStream : public QObject
{
    Q_OBJECT

    Q_PROPERTY(QAbstractVideoSurface *videoSurface READ videoSurface WRITE setVideoSurface)
    Q_PROPERTY(bool active READ active NOTIFY activeChanged)

public:
    explicit VideoStream(QObject *parent = nullptr);
    ~VideoStream() override;

    QAbstractVideoSurface *videoSurface() const { return m_surface; }
    void setVideoSurface(QAbstractVideoSurface *surface);

    bool active() const { return m_active; }

    /// Called from the Qt thread with a decoded frame.
    void present(const QImage &frame);

    /// Same with raw I420 planes: the surface renders YUV and the GPU converts,
    /// which is what makes receiving video affordable on weak devices.
    void presentYuv(const QByteArray &planes, const QSize &size);

    /// Drops the surface's format, for instance when a call ends.
    void stop();

signals:
    void activeChanged();

private:
    void startSurface(const QSize &size, QVideoFrame::PixelFormat format);

    QAbstractVideoSurface *m_surface = nullptr;
    QSize m_size;
    QVideoFrame::PixelFormat m_format = QVideoFrame::Format_Invalid;
    bool m_active = false;
};

#endif // VIDEOSTREAM_H
