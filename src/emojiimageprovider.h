#ifndef EMOJIIMAGEPROVIDER_H
#define EMOJIIMAGEPROVIDER_H

#include <QQuickImageProvider>

class EmojiStore;

/// Hands out emoji pictures, and only ones whose checksum still matches: a file
/// path in QML is opened by QML, and nothing can be put in between.
class EmojiImageProvider : public QQuickImageProvider
{
public:
    explicit EmojiImageProvider(EmojiStore *store);

    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;

private:
    EmojiStore *m_store;
};

#endif // EMOJIIMAGEPROVIDER_H
