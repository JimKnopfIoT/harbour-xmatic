#ifndef EMOJIIMAGEPROVIDER_H
#define EMOJIIMAGEPROVIDER_H

#include <QQuickImageProvider>

class EmojiStore;

/// Hands out emoji pictures, and only ones whose checksum still matches.
///
/// This exists so the check cannot be walked around: a file path in QML is
/// opened by QML, and nothing can be put in between. The id is the picture's
/// name and nothing else - it is checked character by character before a path
/// is built from it, so a name cannot reach out of the directory.
class EmojiImageProvider : public QQuickImageProvider
{
public:
    explicit EmojiImageProvider(EmojiStore *store);

    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;

private:
    EmojiStore *m_store;
};

#endif // EMOJIIMAGEPROVIDER_H
