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
    Q_PROPERTY(bool hideKeyboardOnSend READ hideKeyboardOnSend WRITE setHideKeyboardOnSend
               NOTIFY hideKeyboardOnSendChanged)
    Q_PROPERTY(bool emojiImages READ emojiImages WRITE setEmojiImages
               NOTIFY emojiImagesChanged)
    Q_PROPERTY(QString directoryServer READ directoryServer WRITE setDirectoryServer
               NOTIFY directoryServerChanged)
    Q_PROPERTY(QStringList directoryServers READ directoryServers NOTIFY directoryServersChanged)
    Q_PROPERTY(QString callPolicy READ callPolicy WRITE setCallPolicy NOTIFY callPolicyChanged)
    Q_PROPERTY(bool groupCalls READ groupCalls WRITE setGroupCalls NOTIFY callPolicyChanged)
    Q_PROPERTY(bool videoCalls READ videoCalls WRITE setVideoCalls NOTIFY callPolicyChanged)
    Q_PROPERTY(bool callFlood READ callFlood WRITE setCallFlood NOTIFY callPolicyChanged)
    Q_PROPERTY(bool sendReadReceipts READ sendReadReceipts WRITE setSendReadReceipts
               NOTIFY sendReadReceiptsChanged)
    Q_PROPERTY(QString mediaWipe READ mediaWipe WRITE setMediaWipe NOTIFY mediaWipeChanged)
    Q_PROPERTY(bool unencryptedStorageAccepted READ unencryptedStorageAccepted
               NOTIFY unencryptedStorageAcceptedChanged)
    Q_PROPERTY(QStringList emojiFavourites READ emojiFavourites WRITE setEmojiFavourites
               NOTIFY emojiFavouritesChanged)

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

    /// Who may make this phone ring: "all", "direct" (anybody this account has
    /// a direct chat with) or "list" (only allowedCallers). "direct" by
    /// default - a stranger in a shared room could otherwise ring at will,
    /// and a call rings past every mute.
    QString callPolicy() const;
    void setCallPolicy(const QString &policy);

    /// Whether a call arriving in a room with more than two people is allowed
    /// at all. Off: in such a room every member sees the call id, which is
    /// what makes a call there worth hijacking.
    bool groupCalls() const;
    void setGroupCalls(bool enabled);

    /// Whether anybody may open the camera on this device by offering video.
    /// Off: an offer with video is answered as a voice call.
    bool videoCalls() const;
    void setVideoCalls(bool enabled);

    /// Whether a caller has to wait between two calls. Off by default: it
    /// costs a genuine second attempt, and most people never need it.
    bool callFlood() const;
    void setCallFlood(bool enabled);

    /// Both lists name people and live encrypted in the core since 0.22.2.
    /// What is left here is read once, moved over and removed.
    QStringList legacyAllowedCallers() const;
    QStringList legacyTrustedRecipients() const;
    void dropLegacyLists();

    /// Whether this device tells the others what it has read. On by default,
    /// which is what it always did; off keeps the reading to itself.
    bool sendReadReceipts() const;
    void setSendReadReceipts(bool enabled);

    /// Whether the user has explicitly said this device may run without
    /// encrypted local storage.
    ///
    /// Only ever true because somebody was shown what it costs and tapped the
    /// second button. The failure direction is deliberate: a value that cannot
    /// be read counts as "not accepted", because asking again is harmless
    /// while assuming consent is not.
    bool unencryptedStorageAccepted() const;
    Q_INVOKABLE void acceptUnencryptedStorage();

    /// When the downloaded media go: "never", "logout" (the default), "exit"
    /// or "background". "never" means never, sign-out included.
    QString mediaWipe() const;
    void setMediaWipe(const QString &when);

    /// Whether a link in a message can be tapped.
    bool clickableLinks() const;
    void setClickableLinks(bool enabled);

    /// Whether the microphone sits next to the message field. On by default -
    /// it is what the feature has always been - but it is one hold away from a
    /// recording, and not everybody wants that within reach.
    bool voiceMessages() const;
    void setVoiceMessages(bool enabled);

    /// Whether the keyboard goes away once a message is sent, the way the
    /// big messengers do it. On by default - asked for, and the conversation
    /// is what one wants to see after sending, not the field one has just
    /// emptied. Off keeps the keyboard up for the next message.
    bool hideKeyboardOnSend() const;
    void setHideKeyboardOnSend(bool enabled);

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

    /// The emoji the picker offers first. Empty and never written are two
    /// different states: never written means the built-in handful is shown,
    /// written and empty means the user has taken them all out. The list the
    /// picker starts from lives in `qml/pages/Emoji.js`, not here - this only
    /// keeps what the user made of it.
    QStringList emojiFavourites() const;
    void setEmojiFavourites(const QStringList &keys);
    Q_INVOKABLE bool hasEmojiFavourites() const;

    /// Recipients the user chose not to be warned about again before sending
    /// into an encrypted room.

    /// Clears them all and answers how many there were - the warning promised
    /// "until you reset it" and needs somewhere to do that.


signals:
    void startPageChanged();
    void notificationPreviewChanged();
    void showReadStatusChanged();
    void clickableLinksChanged();
    void voiceMessagesChanged();
    void hideKeyboardOnSendChanged();
    void emojiImagesChanged();
    void directoryServerChanged();
    void directoryServersChanged();
    void callPolicyChanged();
    void sendReadReceiptsChanged();
    void mediaWipeChanged();
    void unencryptedStorageAcceptedChanged();
    void emojiFavouritesChanged();
};

#endif // APPSETTINGS_H
