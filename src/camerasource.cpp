#include "camerasource.h"

#include <QCamera>
#include <QCameraInfo>
#include <QCameraViewfinderSettings>
#include <QVideoFrame>

namespace {

/// Maps Qt's frame layout to GStreamer's name for the same bytes. Anything
/// unlisted is refused: a wrong format looks like noise, which is harder to see.
QString gstFormatFor(QVideoFrame::PixelFormat format)
{
    switch (format) {
    case QVideoFrame::Format_RGB32:
    case QVideoFrame::Format_ARGB32:
        return QStringLiteral("BGRx");
    case QVideoFrame::Format_BGR32:
    case QVideoFrame::Format_BGRA32:
        return QStringLiteral("xRGB");
    case QVideoFrame::Format_RGB24:
        return QStringLiteral("RGB");
    case QVideoFrame::Format_BGR24:
        return QStringLiteral("BGR");
    case QVideoFrame::Format_YUV420P:
        return QStringLiteral("I420");
    case QVideoFrame::Format_NV12:
        return QStringLiteral("NV12");
    case QVideoFrame::Format_NV21:
        return QStringLiteral("NV21");
    case QVideoFrame::Format_UYVY:
        return QStringLiteral("UYVY");
    case QVideoFrame::Format_YUYV:
        return QStringLiteral("YUY2");
    default:
        return QString();
    }
}

/// What a call needs, not what the sensor can do. A 32-bit build means an
/// old SoC that also has to encode this — QVGA is what it can afford.
#ifdef Q_PROCESSOR_ARM_32
const int kCaptureWidth = 320;
const int kCaptureHeight = 240;
#else
const int kCaptureWidth = 640;
const int kCaptureHeight = 480;
#endif

} // namespace

CameraSource::CameraSource(QObject *parent)
    : QAbstractVideoSurface(parent)
{
}

CameraSource::~CameraSource()
{
    stop();
}

QList<QVideoFrame::PixelFormat> CameraSource::supportedPixelFormats(
    QAbstractVideoBuffer::HandleType type) const
{
    if (type != QAbstractVideoBuffer::NoHandle) {
        // Hardware handles cannot be read on the CPU, and the frames have to
        // reach the encoder as plain memory.
        return QList<QVideoFrame::PixelFormat>();
    }

    return QList<QVideoFrame::PixelFormat>() << QVideoFrame::Format_NV21
                                             << QVideoFrame::Format_NV12
                                             << QVideoFrame::Format_YUV420P
                                             << QVideoFrame::Format_UYVY
                                             << QVideoFrame::Format_YUYV
                                             << QVideoFrame::Format_RGB32
                                             << QVideoFrame::Format_ARGB32
                                             << QVideoFrame::Format_BGR32
                                             << QVideoFrame::Format_RGB24;
}

bool CameraSource::present(const QVideoFrame &frame)
{
    QVideoFrame readable(frame);
    if (!readable.map(QAbstractVideoBuffer::ReadOnly)) {
        return false;
    }

    const QString format = gstFormatFor(readable.pixelFormat());
    if (format.isEmpty()) {
        readable.unmap();
        return false;
    }

    // Copied: the frame is unmapped as soon as this returns, and the bytes
    // travel on to the encoder.
    const QByteArray data(reinterpret_cast<const char *>(readable.bits()),
                          readable.mappedBytes());
    const int width = readable.width();
    const int height = readable.height();
    readable.unmap();

    emit frameReady(data, width, height, format);
    return true;
}

bool CameraSource::isAvailable()
{
    // An empty list is not "no camera": the adaptation layer provides a default
    // device without registering it. Only a default that refuses counts as absent.
    if (!QCameraInfo::availableCameras().isEmpty()) {
        return true;
    }
    return !QCameraInfo::defaultCamera().isNull();
}

void CameraSource::start()
{
    if (m_camera) {
        return;
    }

    // The front camera is the one a video call wants.
    QCameraInfo chosen;
    const QList<QCameraInfo> cameras = QCameraInfo::availableCameras();
    for (const QCameraInfo &camera : cameras) {
        if (camera.position() == QCamera::FrontFace) {
            chosen = camera;
            break;
        }
    }
    if (chosen.isNull() && !cameras.isEmpty()) {
        chosen = cameras.first();
    }

    if (chosen.isNull()) {
        // Nothing enumerated: fall back to whatever the platform considers its
        // default rather than declaring the device camera-less.
        qInfo("xmatic: no camera enumerated, trying the default device");
        m_camera = new QCamera(this);
    } else {
        m_camera = new QCamera(chosen, this);
    }

    m_camera->setCaptureMode(QCamera::CaptureVideo);
    m_camera->setViewfinder(this);

    connect(m_camera,
            static_cast<void (QCamera::*)(QCamera::Error)>(&QCamera::error),
            this,
            [this](QCamera::Error) { emit failed(m_camera->errorString()); });

    // The sensor offers far more than a call needs, and the resolution has to be
    // one it actually offers - an unsupported one kills the whole pipeline.
    m_camera->load();

    const QList<QSize> resolutions = m_camera->supportedViewfinderResolutions();
    QSize best;
    for (const QSize &size : resolutions) {
        const bool smallEnough = size.width() <= kCaptureWidth * 2;
        if (!smallEnough) {
            continue;
        }
        if (!best.isValid() || size.width() > best.width()) {
            best = size;
        }
    }
    if (!best.isValid() && !resolutions.isEmpty()) {
        // Nothing modest on offer: take the smallest there is.
        best = resolutions.first();
        for (const QSize &size : resolutions) {
            if (size.width() < best.width()) {
                best = size;
            }
        }
    }

    if (best.isValid()) {
        QCameraViewfinderSettings settings;
        settings.setResolution(best);
        m_camera->setViewfinderSettings(settings);
        qInfo("xmatic: camera resolution %dx%d chosen from %d offered",
              best.width(),
              best.height(),
              resolutions.size());
    } else {
        // No list offered on some ports: ask for plain VGA rather than swallowing the
        // sensor default, which no 32-bit CPU encodes in real time.
        QCameraViewfinderSettings settings;
        settings.setResolution(QSize(kCaptureWidth, kCaptureHeight));
        m_camera->setViewfinderSettings(settings);
        qInfo("xmatic: camera offers no resolution list, requesting %dx%d",
              kCaptureWidth,
              kCaptureHeight);
    }

    m_camera->start();
    qInfo("xmatic: camera started: %s",
          chosen.isNull() ? "default device" : qPrintable(chosen.description()));
}

void CameraSource::stop()
{
    if (!m_camera) {
        return;
    }

    // Detach first, so nothing is posted to a surface being torn down - and never
    // hand the camera a null viewfinder.
    QCamera *camera = m_camera;
    m_camera = nullptr;

    // Disconnected before it is stopped: the error handler reads `m_camera`, which
    // is already null. That order was a segfault on hanging up.
    disconnect(camera, nullptr, this, nullptr);
    camera->stop();
    camera->deleteLater();

    if (isActive()) {
        QAbstractVideoSurface::stop();
    }
}
