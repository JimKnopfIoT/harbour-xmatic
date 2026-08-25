#ifndef EMOJISET_H
#define EMOJISET_H

#include <QByteArray>
#include <QFutureWatcher>
#include <QHash>
#include <QObject>
#include <QString>

class EmojiStore;

/// Reading a folder of emoji pictures in, once, under supervision.
///
/// Nothing is downloaded and nothing is shipped: the user brings a set. What
/// happens to it here is the whole point of the exercise -
///
/// * only files whose name is code points in hex are considered at all,
/// * each one has to be small enough and has to decode,
/// * each one is rasterised to PNG, so the SVG parser runs exactly once, in
///   this controlled moment, and never again while a conversation is drawn,
/// * each written picture's checksum is recorded, and the image provider
///   checks it every time the picture is used.
class EmojiSet : public QObject
{
    Q_OBJECT

    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    Q_PROPERTY(bool verified READ verified NOTIFY statusChanged)
    Q_PROPERTY(bool tampered READ tampered NOTIFY statusChanged)
    Q_PROPERTY(int count READ count NOTIFY statusChanged)
    Q_PROPERTY(int lastImported READ lastImported NOTIFY statusChanged)
    Q_PROPERTY(int lastRejected READ lastRejected NOTIFY statusChanged)

public:
    explicit EmojiSet(EmojiStore *store, QObject *parent = nullptr);

    bool busy() const { return m_busy; }
    bool verified() const;
    bool tampered() const;
    int count() const;
    int lastImported() const { return m_lastImported; }
    int lastRejected() const { return m_lastRejected; }

    /// Reads every usable picture out of that folder. The url is what a
    /// Sailfish folder picker hands back.
    Q_INVOKABLE void importFrom(const QString &folderUrl);

    /// Throws the set away again, checksums and pictures both.
    Q_INVOKABLE void removeAll();

signals:
    void busyChanged();
    void statusChanged();

private:
    struct Outcome {
        int imported = 0;
        int rejected = 0;
        /// The run never got as far as reading. The old set and its checksums
        /// have to stay: adopting an empty list would silently drop the
        /// verified set back to the unchecked path.
        bool aborted = false;
        QHash<QString, QByteArray> checksums;
    };

    void adopt();

    EmojiStore *m_store;
    QFutureWatcher<Outcome> m_watcher;
    bool m_busy = false;
    int m_lastImported = 0;
    int m_lastRejected = 0;
};

#endif // EMOJISET_H
