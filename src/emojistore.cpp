#include "emojistore.h"

#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMutexLocker>

namespace {
/// The same ceiling the unchecked path uses: a picture for a character in a
/// chat line has no business being larger, and it bounds what the decoder is
/// ever handed.
const qint64 MaximumPictureBytes = 64 * 1024;
}

EmojiStore::EmojiStore(const QString &directory, QObject *parent)
    : QObject(parent)
    , m_directory(directory)
    , m_manifestPath(QFileInfo(directory).absolutePath() + QStringLiteral("/emoji-manifest.json"))
{
    reload();
}

void EmojiStore::reload()
{
    QHash<QString, QByteArray> checksums;

    QFile file(m_manifestPath);
    // Read at start, on the main thread: a file of any size there would be an
    // out-of-memory at launch. One entry is about sixty bytes.
    if (file.open(QIODevice::ReadOnly) && file.size() <= 4 * 1024 * 1024) {
        const QJsonObject root = QJsonDocument::fromJson(file.readAll()).object();
        const QJsonObject files = root.value(QStringLiteral("files")).toObject();
        for (auto it = files.constBegin(); it != files.constEnd(); ++it) {
            const QString name = it.key();
            const QByteArray sum = it.value().toString().toLatin1();
            // A name with a path in it never gets as far as being looked up.
            if (name.contains(QLatin1Char('/')) || name.contains(QLatin1Char('\\'))
                || sum.isEmpty()) {
                continue;
            }
            checksums.insert(name, sum);
        }
    }

    QMutexLocker locked(&m_lock);
    m_checksums = checksums;
    m_tampered = false;
}

bool EmojiStore::verified() const
{
    QMutexLocker locked(&m_lock);
    return !m_checksums.isEmpty();
}

int EmojiStore::count() const
{
    QMutexLocker locked(&m_lock);
    return m_checksums.count();
}

bool EmojiStore::tampered() const
{
    QMutexLocker locked(&m_lock);
    return m_tampered;
}

bool EmojiStore::knows(const QString &fileName) const
{
    QMutexLocker locked(&m_lock);
    return !m_tampered && m_checksums.contains(fileName);
}

QByteArray EmojiStore::verifiedBytes(const QString &fileName)
{
    QByteArray expected;
    {
        QMutexLocker locked(&m_lock);
        if (m_tampered) {
            return QByteArray();
        }
        expected = m_checksums.value(fileName);
    }
    if (expected.isEmpty()) {
        return QByteArray();
    }

    QFile file(m_directory + QLatin1Char('/') + fileName);
    QFileInfo info(file);
    if (!info.exists() || !info.isFile() || info.size() > MaximumPictureBytes) {
        // A picture that is gone is a missing one, not a swapped one: the
        // character is drawn and nothing is refused.
        return QByteArray();
    }
    if (!file.open(QIODevice::ReadOnly)) {
        return QByteArray();
    }
    const QByteArray bytes = file.readAll();
    const QByteArray actual = QCryptographicHash::hash(bytes, QCryptographicHash::Md5).toHex();
    if (actual != expected) {
        {
            QMutexLocker locked(&m_lock);
            m_tampered = true;
        }
        emit tamperedChanged();
        return QByteArray();
    }
    return bytes;
}

void EmojiStore::adopt(const QHash<QString, QByteArray> &checksums)
{
    QJsonObject files;
    for (auto it = checksums.constBegin(); it != checksums.constEnd(); ++it) {
        files.insert(it.key(), QString::fromLatin1(it.value()));
    }
    QJsonObject root;
    root.insert(QStringLiteral("version"), 1);
    root.insert(QStringLiteral("files"), files);

    QFile file(m_manifestPath);
    if (file.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        file.write(QJsonDocument(root).toJson(QJsonDocument::Compact));
        file.close();
    }

    {
        QMutexLocker locked(&m_lock);
        m_checksums = checksums;
        m_tampered = false;
    }
    emit tamperedChanged();
    emit contentChanged();
}

void EmojiStore::forget()
{
    QFile::remove(m_manifestPath);
    QDir(m_directory).removeRecursively();

    {
        QMutexLocker locked(&m_lock);
        m_checksums.clear();
        m_tampered = false;
    }
    emit tamperedChanged();
    emit contentChanged();
}
