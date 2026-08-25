#include "emojiset.h"

#include <QBuffer>
#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QImage>
#include <QImageReader>
#include <QPainter>
#include <QUrl>
#include <QtConcurrent>

#include "emojistore.h"

namespace {

/// What one picture may cost, before and after. The same ceiling the display
/// path uses; a picture for a character in a chat line has no business being
/// larger.
const qint64 MaximumSourceBytes = 64 * 1024;
/// What a picture is rasterised to. Covers the largest icon size any of the
/// shipped screen densities asks for, and keeps a full set at a size a phone
/// can spare.
const int RenderSize = 128;
/// A source image larger than this is refused before it is decoded: the
/// decoder allocates width * height * 4 bytes, and that number must not come
/// from the file.
const int MaximumSourcePixels = 2048;

/// A picture's name has to be the emoji's code points in hex, joined by
/// dashes. That is the naming every emoji set uses, and it is also what makes
/// the name safe to build a path from.
bool nameIsSound(const QString &base)
{
    if (base.isEmpty() || base.length() > 64) {
        return false;
    }
    bool group = false;
    for (const QChar c : base) {
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

/// Reads one file and writes the rasterised picture. Returns its checksum, or
/// empty when the file is not what it claims to be.
QByteArray convert(const QFileInfo &source, const QString &targetPath)
{
    QImageReader reader(source.absoluteFilePath());
    // The format comes from the suffix, not from the content: this way only
    // the two decoders that were meant can ever be reached.
    const QByteArray format = source.suffix().toLower().toLatin1();
    if (format != "svg" && format != "png") {
        return QByteArray();
    }
    reader.setFormat(format);
    if (!reader.canRead()) {
        return QByteArray();
    }

    if (format == "svg") {
        // Qt 5.6 has no limit on entity expansion, so a kilobyte of XML can
        // ask for gigabytes of memory. A picture of an emoji needs neither
        // entities nor a doctype.
        QFile raw(source.absoluteFilePath());
        if (!raw.open(QIODevice::ReadOnly)) {
            return QByteArray();
        }
        const QByteArray head = raw.read(MaximumSourceBytes);
        if (head.contains("<!ENTITY") || head.contains("<!DOCTYPE")) {
            return QByteArray();
        }
    }

    if (format == "png") {
        // A size the reader cannot state is a size that cannot be checked.
        const QSize declared = reader.size();
        if (!declared.isValid() || declared.width() > MaximumSourcePixels
            || declared.height() > MaximumSourcePixels) {
            return QByteArray();
        }
    }

    // Scalable art is asked for at the size it is wanted; a bitmap is read as
    // it is and scaled below.
    if (format == "svg") {
        reader.setScaledSize(QSize(RenderSize, RenderSize));
    }

    const QImage decoded = reader.read();
    if (decoded.isNull()) {
        return QByteArray();
    }

    // One square canvas for every picture, so the rows in a chat line up
    // whatever the set's own proportions are.
    QImage canvas(RenderSize, RenderSize, QImage::Format_ARGB32_Premultiplied);
    canvas.fill(Qt::transparent);
    {
        const QImage fitted = decoded.scaled(RenderSize, RenderSize, Qt::KeepAspectRatio,
                                             Qt::SmoothTransformation);
        QPainter painter(&canvas);
        painter.drawImage((RenderSize - fitted.width()) / 2,
                          (RenderSize - fitted.height()) / 2, fitted);
    }

    // Encoded once, in memory, and the checksum is taken from those very
    // bytes. Hashing the file back off the disk would leave a window in which
    // somebody else's bytes get recorded as the trusted ones.
    QByteArray bytes;
    {
        QBuffer buffer(&bytes);
        buffer.open(QIODevice::WriteOnly);
        if (!canvas.save(&buffer, "PNG")) {
            return QByteArray();
        }
    }
    if (bytes.isEmpty() || bytes.size() > MaximumSourceBytes) {
        return QByteArray();
    }

    QFile target(targetPath);
    if (!target.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        return QByteArray();
    }
    if (target.write(bytes) != bytes.size()) {
        target.close();
        QFile::remove(targetPath);
        return QByteArray();
    }
    target.close();

    return QCryptographicHash::hash(bytes, QCryptographicHash::Md5).toHex();
}

}

EmojiSet::EmojiSet(EmojiStore *store, QObject *parent)
    : QObject(parent)
    , m_store(store)
{
    connect(&m_watcher, &QFutureWatcher<Outcome>::finished, this, &EmojiSet::adopt);
    if (m_store) {
        // The provider notices a swapped picture on the image thread; the page
        // showing the red line lives on this one.
        connect(m_store, &EmojiStore::tamperedChanged, this, &EmojiSet::statusChanged,
                Qt::QueuedConnection);
        connect(m_store, &EmojiStore::contentChanged, this, &EmojiSet::statusChanged,
                Qt::QueuedConnection);
    }
}

bool EmojiSet::verified() const
{
    return m_store && m_store->verified();
}

bool EmojiSet::tampered() const
{
    return m_store && m_store->tampered();
}

int EmojiSet::count() const
{
    return m_store ? m_store->count() : 0;
}

void EmojiSet::importFrom(const QString &folderUrl)
{
    if (m_busy || !m_store) {
        return;
    }

    const QString folder = folderUrl.startsWith(QStringLiteral("file://"))
            ? QUrl(folderUrl).toLocalFile()
            : folderUrl;
    if (folder.isEmpty() || !QFileInfo(folder).isDir()) {
        return;
    }

    const QString target = m_store->directory();

    m_busy = true;
    emit busyChanged();

    m_watcher.setFuture(QtConcurrent::run([folder, target]() {
        Outcome outcome;

        // Reading the set out of the very directory it is written to is the
        // obvious thing for somebody who copied it there by hand - and it
        // would delete the source before reading a single file. The old
        // directory is therefore moved aside first and read from there.
        QString readFrom = folder;
        QString aside;
        // Canonical, not absolute: a link pointing at the target directory
        // compares equal only after it is resolved, and getting that wrong
        // deletes the source.
        const QFileInfo folderInfo(folder);
        const QFileInfo targetInfo(target);
        const QString folderPath = folderInfo.canonicalFilePath().isEmpty()
                ? QDir(folder).absolutePath() : folderInfo.canonicalFilePath();
        const QString targetPath = targetInfo.canonicalFilePath().isEmpty()
                ? QDir(target).absolutePath() : targetInfo.canonicalFilePath();
        if (folderPath == targetPath || folderPath.startsWith(targetPath + QLatin1Char('/'))) {
            aside = targetPath + QStringLiteral(".reading");
            // A previous run was interrupted between moving the set aside and
            // writing it back. That directory is then the only copy.
            if (!QDir(aside).exists() && !QDir().rename(targetPath, aside)) {
                outcome.aborted = true;
                return outcome;
            }
            readFrom = folderPath == targetPath
                    ? aside
                    : aside + folderPath.mid(targetPath.length());
        } else {
            // The old set goes first, whatever it was: leaving it behind would
            // mean a directory half checked and half not.
            QDir(target).removeRecursively();
        }
        QDir().mkpath(target);

        QDir source(readFrom);
        const QFileInfoList files = source.entryInfoList(
            QStringList() << QStringLiteral("*.svg") << QStringLiteral("*.png"),
            QDir::Files | QDir::NoSymLinks);

        for (const QFileInfo &file : files) {
            const QString base = file.completeBaseName().toLower();
            if (!nameIsSound(base) || file.size() > MaximumSourceBytes) {
                ++outcome.rejected;
                continue;
            }
            const QString targetPath = target + QLatin1Char('/') + base + QStringLiteral(".png");
            const QByteArray sum = convert(file, targetPath);
            if (sum.isEmpty()) {
                ++outcome.rejected;
                continue;
            }
            outcome.checksums.insert(base + QStringLiteral(".png"), sum);
            ++outcome.imported;
        }

        if (!aside.isEmpty()) {
            QDir(aside).removeRecursively();
        }

        return outcome;
    }));
}

void EmojiSet::removeAll()
{
    if (m_busy || !m_store) {
        return;
    }
    m_store->forget();
    m_lastImported = 0;
    m_lastRejected = 0;
    emit statusChanged();
}

void EmojiSet::adopt()
{
    const Outcome outcome = m_watcher.result();
    m_lastImported = outcome.imported;
    m_lastRejected = outcome.rejected;

    // An aborted run has read nothing; the set on disk and its checksums are
    // still the ones that were verified.
    if (m_store && !outcome.aborted) {
        m_store->adopt(outcome.checksums);
    }

    m_busy = false;
    emit busyChanged();
    emit statusChanged();
}
