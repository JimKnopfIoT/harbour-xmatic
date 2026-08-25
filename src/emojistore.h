#ifndef EMOJISTORE_H
#define EMOJISTORE_H

#include <QByteArray>
#include <QHash>
#include <QMutex>
#include <QObject>
#include <QString>

/// The checked set of emoji pictures, shared between the importer and the
/// image provider.
///
/// The importer writes the pictures and a list of their checksums; the
/// provider verifies a picture against that list every time it hands one to
/// the decoder. What this buys and what it does not:
///
/// * It catches a picture that changed after it was read in - by accident, by
///   another process, by a hand at a shell. The set is then refused as a whole
///   and the conversation falls back to drawing the characters.
/// * It does not stop somebody who can already write this app's data
///   directory: they can rewrite the list along with the pictures. The lasting
///   protection is that the importer rasterises everything to PNG, so the SVG
///   parser never runs again at display time.
///
/// Every method is callable from any thread; the provider runs on Qt's image
/// thread.
class EmojiStore : public QObject
{
    Q_OBJECT

public:
    explicit EmojiStore(const QString &directory, QObject *parent = nullptr);

    /// Where the pictures live.
    QString directory() const { return m_directory; }
    /// Where the checksums live. Beside the pictures, not among them, so a
    /// file dropped into the set cannot be named like the list.
    QString manifestPath() const { return m_manifestPath; }

    /// Reads the checksum list from disk. Also the way back after an import.
    void reload();

    /// Whether a checked set exists at all. Without one the app still shows
    /// pictures a user copied in by hand - unchecked, the way it always did.
    bool verified() const;
    /// How many pictures the list holds.
    int count() const;
    /// Whether a picture failed its check since the app started. Latching:
    /// once something did not match, the set stays refused until it is read in
    /// again.
    bool tampered() const;

    /// Whether the checked set holds this picture (file name without path).
    bool knows(const QString &fileName) const;

    /// The bytes of a picture, but only if its checksum still matches.
    /// Empty on any doubt, and the first doubt refuses the whole set.
    QByteArray verifiedBytes(const QString &fileName);

    /// Replaces the list after an import.
    void adopt(const QHash<QString, QByteArray> &checksums);
    /// Forgets everything, for "remove pictures".
    void forget();

signals:
    /// From the image thread; connect queued.
    void tamperedChanged();
    void contentChanged();

private:
    QString m_directory;
    QString m_manifestPath;

    mutable QMutex m_lock;
    QHash<QString, QByteArray> m_checksums;
    bool m_tampered = false;
};

#endif // EMOJISTORE_H
