#include "appsettings.h"

#include <QDir>
#include <QFileInfo>
#include <QSettings>
#include <QStandardPaths>
#include <QUrl>

namespace {

/// The settings file, with its directory created. QSettings cannot be handed
/// back from a function - it is a QObject - so this returns the path.
QString writablePath()
{
    const QString path = appSettingsPath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    return path;
}

/// Writes and says so in the journal if the sandbox refused - a setting that
/// silently never persists is the failure this app already had once.
void store(QSettings &settings, const QString &key, const QVariant &value, const char *what)
{
    settings.setValue(key, value);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not save %s (status %d)", what,
                 static_cast<int>(settings.status()));
    }
}

/// A directory server as it is stored: host name only, no scheme, no path.
QString normalizedServerName(const QString &server)
{
    QString name = server.trimmed();
    if (name.isEmpty()) {
        return name;
    }
    if (name.contains(QStringLiteral("://"))) {
        const QUrl url(name);
        name = url.host();
    }
    while (name.endsWith(QLatin1Char('/'))) {
        name.chop(1);
    }
    return name;
}

} // namespace

QString appSettingsPath()
{
    return QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation)
           + QStringLiteral("/settings.conf");
}

AppSettings::AppSettings(QObject *parent)
    : QObject(parent)
{
}

QString AppSettings::startPage() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    const QString page = settings.value(QStringLiteral("ui/startPage"),
                                        QStringLiteral("rooms")).toString();
    return page == QLatin1String("spaces") ? page : QStringLiteral("rooms");
}

void AppSettings::setStartPage(const QString &page)
{
    const QString wanted = page == QLatin1String("spaces") ? QStringLiteral("spaces")
                                                           : QStringLiteral("rooms");
    if (wanted == startPage()) {
        return;
    }
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("ui/startPage"), wanted, "the start page");
    if (settings.status() == QSettings::NoError) {
        qInfo("xmatic: start page set to %s", qPrintable(wanted));
    }
    emit startPageChanged();
}

bool AppSettings::notificationPreview() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("ui/notificationPreview"), false).toBool();
}

void AppSettings::setNotificationPreview(bool enabled)
{
    if (enabled == notificationPreview()) {
        return;
    }
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("ui/notificationPreview"), enabled,
          "the notification preview setting");
    if (settings.status() == QSettings::NoError) {
        qInfo("xmatic: notification preview %s", enabled ? "on" : "off");
    }
    emit notificationPreviewChanged();
}

bool AppSettings::showReadStatus() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("ui/showReadStatus"), false).toBool();
}

void AppSettings::setShowReadStatus(bool enabled)
{
    if (enabled == showReadStatus()) {
        return;
    }
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("ui/showReadStatus"), enabled, "the read status setting");
    if (settings.status() == QSettings::NoError) {
        qInfo("xmatic: read status %s", enabled ? "on" : "off");
    }
    // The open timeline still runs with the old setting; rebuilding it is the
    // bridge's job, which listens for this.
    emit showReadStatusChanged();
}

bool AppSettings::clickableLinks() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("ui/clickableLinks"), false).toBool();
}

void AppSettings::setClickableLinks(bool enabled)
{
    if (enabled == clickableLinks()) {
        return;
    }
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("ui/clickableLinks"), enabled,
          "the clickable links setting");
    if (settings.status() == QSettings::NoError) {
        qInfo("xmatic: clickable links %s", enabled ? "on" : "off");
    }
    emit clickableLinksChanged();
}

bool AppSettings::voiceMessages() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("ui/voiceMessages"), true).toBool();
}

void AppSettings::setVoiceMessages(bool enabled)
{
    if (enabled == voiceMessages()) {
        return;
    }
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("ui/voiceMessages"), enabled, "the voice message setting");
    if (settings.status() == QSettings::NoError) {
        qInfo("xmatic: voice messages %s", enabled ? "on" : "off");
    }
    emit voiceMessagesChanged();
}

bool AppSettings::emojiImages() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("ui/emojiImages"), false).toBool();
}

void AppSettings::setEmojiImages(bool enabled)
{
    if (enabled == emojiImages()) {
        return;
    }
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("ui/emojiImages"), enabled, "the emoji image setting");
    if (settings.status() == QSettings::NoError) {
        qInfo("xmatic: emoji images %s", enabled ? "on" : "off");
    }
    emit emojiImagesChanged();
}

QString AppSettings::directoryServer() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("directory/server")).toString();
}

void AppSettings::setDirectoryServer(const QString &server)
{
    const QString wanted = normalizedServerName(server);
    if (wanted == directoryServer()) {
        return;
    }
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("directory/server"), wanted, "the directory server");
    emit directoryServerChanged();
}

QStringList AppSettings::directoryServers() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("directory/servers")).toStringList();
}

void AppSettings::addDirectoryServer(const QString &server)
{
    const QString name = normalizedServerName(server);
    if (name.isEmpty()) {
        return;
    }
    QStringList servers = directoryServers();
    if (servers.contains(name)) {
        return;
    }
    servers.append(name);
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("directory/servers"), servers, "the directory servers");
    emit directoryServersChanged();
}

void AppSettings::removeDirectoryServer(const QString &server)
{
    const QString name = normalizedServerName(server);
    QStringList servers = directoryServers();
    if (servers.removeAll(name) == 0) {
        return;
    }
    QSettings settings(writablePath(), QSettings::IniFormat);
    store(settings, QStringLiteral("directory/servers"), servers, "the directory servers");
    emit directoryServersChanged();
}

bool AppSettings::isRecipientTrusted(const QString &userId) const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    return settings.value(QStringLiteral("security/trustedRecipients"))
        .toStringList()
        .contains(userId);
}

void AppSettings::trustRecipient(const QString &userId)
{
    QSettings settings(writablePath(), QSettings::IniFormat);
    QStringList trusted =
        settings.value(QStringLiteral("security/trustedRecipients")).toStringList();
    if (trusted.contains(userId)) {
        return;
    }
    trusted.append(userId);
    store(settings, QStringLiteral("security/trustedRecipients"), trusted,
          "the trusted recipient");
}

int AppSettings::resetRecipientWarnings()
{
    QSettings settings(writablePath(), QSettings::IniFormat);
    const int count = settings.value(QStringLiteral("security/trustedRecipients"))
                          .toStringList()
                          .count();
    if (count == 0) {
        return 0;
    }
    settings.remove(QStringLiteral("security/trustedRecipients"));
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not clear the trusted recipients");
        return 0;
    }
    // A count, never the addresses: this line ends up in the journal.
    qInfo("xmatic: %d suppressed recipient warnings cleared", count);
    return count;
}
