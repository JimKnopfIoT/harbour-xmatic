#include "emojiimageprovider.h"

#include <QImage>

#include "emojistore.h"

namespace {
/// A picture's name is code points in hex joined by dashes, nothing else - the
/// importer builds it, so anything that deviates was not written by it.
bool nameIsSound(const QString &id)
{
    if (id.isEmpty() || id.length() > 64) {
        return false;
    }
    bool group = false;
    for (const QChar c : id) {
        if ((c >= QLatin1Char('0') && c <= QLatin1Char('9'))
            || (c >= QLatin1Char('a') && c <= QLatin1Char('f'))) {
            group = true;
            continue;
        }
        if (c == QLatin1Char('-') && group) {
            group = false;
            continue;
        }
        return false;
    }
    return group;
}
}

EmojiImageProvider::EmojiImageProvider(EmojiStore *store)
    : QQuickImageProvider(QQuickImageProvider::Image)
    , m_store(store)
{
}

QImage EmojiImageProvider::requestImage(const QString &id, QSize *size, const QSize &requestedSize)
{
    if (!m_store || !nameIsSound(id)) {
        return QImage();
    }

    const QByteArray bytes = m_store->verifiedBytes(id + QStringLiteral(".png"));
    if (bytes.isEmpty()) {
        return QImage();
    }

    QImage image;
    // The format is not taken from the file: the importer writes PNG, so anything
    // claiming otherwise is refused rather than handed to another decoder.
    if (!image.loadFromData(bytes, "PNG")) {
        return QImage();
    }

    if (size) {
        *size = image.size();
    }
    if (requestedSize.isValid() && requestedSize != image.size()) {
        return image.scaled(requestedSize, Qt::KeepAspectRatio, Qt::SmoothTransformation);
    }
    return image;
}
