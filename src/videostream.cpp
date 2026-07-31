#include "videostream.h"

#include <cstring>

#include <QVideoFrame>
#include <QVideoSurfaceFormat>

VideoStream::VideoStream(QObject *parent)
    : QObject(parent)
{
}

VideoStream::~VideoStream()
{
    stop();
}

void VideoStream::setVideoSurface(QAbstractVideoSurface *surface)
{
    if (m_surface == surface) {
        return;
    }

    if (m_surface && m_surface->isActive()) {
        m_surface->stop();
    }
    m_surface = surface;
    m_size = QSize();
    m_format = QVideoFrame::Format_Invalid;
}

void VideoStream::startSurface(const QSize &size, QVideoFrame::PixelFormat format)
{
    if (!m_surface) {
        return;
    }

    if (m_surface->isActive()) {
        m_surface->stop();
    }

    QVideoSurfaceFormat wanted(size, format);
    if (!m_surface->start(wanted)) {
        // Happens when the QML video node cannot shade this format; the
        // caller may retry with another one.
        qWarning("xmatic: video surface refused format %d", format);
        m_format = QVideoFrame::Format_Invalid;
        return;
    }

    m_size = size;
    m_format = format;
    if (!m_active) {
        m_active = true;
        emit activeChanged();
    }
}

void VideoStream::present(const QImage &frame)
{
    if (!m_surface || frame.isNull()) {
        return;
    }

    // The first frame decides the format, and a resolution change mid-call
    // means the surface has to be restarted.
    if (frame.size() != m_size || m_format != QVideoFrame::Format_RGB32
        || !m_surface->isActive()) {
        startSurface(frame.size(), QVideoFrame::Format_RGB32);
        if (!m_surface->isActive()) {
            return;
        }
    }

    // Already-RGB32 images pass through; convertToFormat is a no-op copy
    // then. Anything else gets one conversion here.
    QVideoFrame video(frame.convertToFormat(QImage::Format_RGB32));
    m_surface->present(video);
}

void VideoStream::presentYuv(const QByteArray &planes, const QSize &size)
{
    if (!m_surface || planes.isEmpty() || size.isEmpty()) {
        return;
    }

    // Tightly packed I420: luma plane plus two quarter-size chroma planes.
    // The pipeline negotiates even dimensions, so this arithmetic is exact;
    // a decoder that pads its strides would deliver more bytes, which is
    // refused rather than displayed as garbage.
    const int expected = size.width() * size.height() * 3 / 2;
    if (planes.size() != expected) {
        static bool warned = false;
        if (!warned) {
            warned = true;
            qWarning("xmatic: YUV frame has %d bytes, expected %d — strides are padded?",
                     planes.size(),
                     expected);
        }
        return;
    }

    if (size != m_size || m_format != QVideoFrame::Format_YUV420P
        || !m_surface->isActive()) {
        startSurface(size, QVideoFrame::Format_YUV420P);
        if (!m_surface->isActive()) {
            return;
        }
    }

    QVideoFrame video(expected, size, size.width(), QVideoFrame::Format_YUV420P);
    if (!video.map(QAbstractVideoBuffer::WriteOnly)) {
        return;
    }
    memcpy(video.bits(), planes.constData(), static_cast<size_t>(expected));
    video.unmap();

    m_surface->present(video);
}

void VideoStream::stop()
{
    if (m_surface && m_surface->isActive()) {
        m_surface->stop();
    }
    m_size = QSize();
    m_format = QVideoFrame::Format_Invalid;

    if (m_active) {
        m_active = false;
        emit activeChanged();
    }
}
