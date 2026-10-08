#ifndef CAMERASOURCE_H
#define CAMERASOURCE_H

#include <QAbstractVideoSurface>
#include <QAtomicInt>
#include <QByteArray>
#include <QSet>
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

    bool start(const QVideoSurfaceFormat &format) override;
    bool present(const QVideoFrame &frame) override;

    /// Whether a camera can be opened at all. Sailfish does not always
    /// enumerate its cameras, so the default device counts as available.
    static bool isAvailable();

    /// Opens the front camera and starts delivering frames. Not start()/stop():
    /// the surface's stop() is virtual, and Qt calls it when it renegotiates.
    void open();
    void close();

    /// Whether pictures arrive at all; some devices start the surface and send none.
    bool delivering() const;

signals:
    /// One captured frame, already in a layout GStreamer understands.
    /// `format` is a GStreamer video format name such as "BGRx" or "NV21".
    void frameReady(const QByteArray &data, int width, int height, const QString &format);

    void failed(const QString &message);

private:
    void chooseResolutionAndStart();

    QCamera *m_camera = nullptr;
    bool m_started = false;
    bool m_refusalLogged = false;
    bool m_surfaceStarted = false;
    QAtomicInt m_framesSeen;
    mutable QSet<int> m_askedHandles;
};

#endif // CAMERASOURCE_H
