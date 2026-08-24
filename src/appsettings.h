#ifndef APPSETTINGS_H
#define APPSETTINGS_H

#include <QObject>
#include <QString>
#include <QStringList>

/// Where every persisted setting of this app lives.
///
/// Sailjail only lets the app write inside its own config directory
/// (AppConfigLocation → ~/.config/org.xmatic/xmatic). QSettings' UserScope path
/// sits one level above that and the sandbox blocks it silently, so the file is
/// named explicitly. Shared, so the appearance and language settings write into
/// the same file instead of each deriving the path again.
QString appSettingsPath();

/// The user's preferences, exposed to QML as `settings`.
///
/// Split out of MatrixBridge: none of this talks to the core, and keeping it
/// next to the protocol made one class own both the session and the colour of
/// a switch. What stays in the bridge is anything a setting *causes* - the read
/// status has to rebuild the open timeline, and only the bridge can do that.
class AppSettings : public QObject
{
    Q_OBJECT

    Q_PROPERTY(QString startPage READ startPage WRITE setStartPage NOTIFY startPageChanged)
    Q_PROPERTY(bool notificationPreview READ notificationPreview WRITE setNotificationPreview
               NOTIFY notificationPreviewChanged)
    Q_PROPERTY(bool showReadStatus READ showReadStatus WRITE setShowReadStatus
               NOTIFY showReadStatusChanged)
    Q_PROPERTY(bool clickableLinks READ clickableLinks WRITE setClickableLinks
               NOTIFY clickableLinksChanged)
    Q_PROPERTY(bool voiceMessages READ voiceMessages WRITE setVoiceMessages
               NOTIFY voiceMessagesChanged)
    Q_PROPERTY(bool emojiImages READ emojiImages WRITE setEmojiImages
               NOTIFY emojiImagesChanged)
    Q_PROPERTY(QString directoryServer READ directoryServer WRITE setDirectoryServer
               NOTIFY directoryServerChanged)
    Q_PROPERTY(QStringList directoryServers READ directoryServers NOTIFY directoryServersChanged)

public:
    explicit AppSettings(QObject *parent = nullptr);

    /// Which list the app opens on: "rooms" or "spaces".
    QString startPage() const;
    void setStartPage(const QString &page);

    /// Whether a notification carries the message itself. Off by default: the
    /// banner also shows on the lock screen.
    bool notificationPreview() const;
    void setNotificationPreview(bool enabled);

    /// Whether other people's read status is tracked and shown. Off by
    /// default: each receipt that moves rewrites a timeline row.
    bool showReadStatus() const;
    void setShowReadStatus(bool enabled);

    /// Whether a link in a message can be tapped.
    bool clickableLinks() const;
    void setClickableLinks(bool enabled);

    /// Whether the microphone sits next to the message field. On by default -
    /// it is what the feature has always been - but it is one hold away from a
    /// recording, and not everybody wants that within reach.
    bool voiceMessages() const;
    void setVoiceMessages(bool enabled);

    /// Whether reactions are drawn from image files the user put there. Off by
    /// default, and deliberately so: the characters cost nothing and are always
    /// right, while an image is a file for a decoder to open - see
    /// docs/SECURITY.md. Nothing is shipped and nothing is downloaded; the
    /// files are the user's own doing.
    bool emojiImages() const;
    void setEmojiImages(bool enabled);

    /// The room directory that is searched, and the list of servers to choose
    /// from beyond the built-in ones.
    QString directoryServer() const;
    void setDirectoryServer(const QString &server);
    QStringList directoryServers() const;
    Q_INVOKABLE void addDirectoryServer(const QString &server);
    Q_INVOKABLE void removeDirectoryServer(const QString &server);

    /// Recipients the user chose not to be warned about again before sending
    /// into an encrypted room.
    Q_INVOKABLE bool isRecipientTrusted(const QString &userId) const;
    Q_INVOKABLE void trustRecipient(const QString &userId);
    /// Clears them all and answers how many there were - the warning promised
    /// "until you reset it" and needs somewhere to do that.
    Q_INVOKABLE int resetRecipientWarnings();

signals:
    void startPageChanged();
    void notificationPreviewChanged();
    void showReadStatusChanged();
    void clickableLinksChanged();
    void voiceMessagesChanged();
    void emojiImagesChanged();
    void directoryServerChanged();
    void directoryServersChanged();
};

#endif // APPSETTINGS_H
