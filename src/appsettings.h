#ifndef APPSETTINGS_H
#define APPSETTINGS_H

#include <QObject>
#include <QString>
#include <QStringList>

/// Where every persisted setting lives. Sailjail only allows writes inside the
/// app's own config directory, and QSettings' default path sits above it.
QString appSettingsPath();

/// The user's preferences, as `settings` in QML. Nothing here talks to the
/// core; what a setting *causes* stays in the bridge.
class AppSettings : public QObject
{
    Q_OBJECT

    Q_PROPERTY(QString startPage READ startPage WRITE setStartPage NOTIFY startPageChanged)
    Q_PROPERTY(bool notificationPreview READ notificationPreview WRITE setNotificationPreview
               NOTIFY notificationPreviewChanged)
    Q_PROPERTY(bool showReadStatus READ showReadStatus WRITE setShowReadStatus
               NOTIFY showReadStatusChanged)
    Q_PROPERTY(bool autoLoadMedia READ autoLoadMedia WRITE setAutoLoadMedia
               NOTIFY autoLoadMediaChanged)
    Q_PROPERTY(bool jumpToReadMarker READ jumpToReadMarker WRITE setJumpToReadMarker
               NOTIFY jumpToReadMarkerChanged)
    Q_PROPERTY(bool clickableLinks READ clickableLinks WRITE setClickableLinks
               NOTIFY clickableLinksChanged)
    /// Whether push was turned on. Stored rather than derived: a registration
    /// survives a restart, and the next start has to know one was made.
    Q_PROPERTY(bool pushEnabled READ pushEnabled WRITE setPushEnabled
               NOTIFY pushChanged)
    /// The Matrix push gateway the homeserver posts to. No default: nothing
    /// can guess it, and a wrong one fails silently on the server's side.
    Q_PROPERTY(QString pushGateway READ pushGateway WRITE setPushGateway
               NOTIFY pushChanged)
    Q_PROPERTY(bool voiceMessages READ voiceMessages WRITE setVoiceMessages
               NOTIFY voiceMessagesChanged)
    Q_PROPERTY(bool hideKeyboardOnSend READ hideKeyboardOnSend WRITE setHideKeyboardOnSend
               NOTIFY hideKeyboardOnSendChanged)
    Q_PROPERTY(bool sendByEnter READ sendByEnter WRITE setSendByEnter
               NOTIFY sendByEnterChanged)
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

    /// Whether a picture loads by itself when its row appears. Off, nothing is
    /// fetched until it is tapped - scrolling past is a request to a server.
    bool autoLoadMedia() const;
    void setAutoLoadMedia(bool enabled);

    /// Whether entering a room opens where reading stopped. On by default; the
    /// line is drawn either way.
    bool jumpToReadMarker() const;
    void setJumpToReadMarker(bool enabled);

    /// Who may ring: "all", "direct" or "list". "direct" by default - a stranger
    /// in a shared room could otherwise ring at will, past every mute.
    QString callPolicy() const;
    void setCallPolicy(const QString &policy);

    /// Whether a call in a room with more than two people is allowed. Off: every
    /// member sees the call id there.
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

    /// Removes settings this build no longer reads. Called once at start.
    void dropRetiredKeys();

    /// When the downloaded media go: "never", "logout" (the default), "exit"
    /// or "background". "never" means never, sign-out included.
    QString mediaWipe() const;
    void setMediaWipe(const QString &when);

    /// Whether a link in a message can be tapped.
    bool clickableLinks() const;
    bool pushEnabled() const;
    void setPushEnabled(bool enabled);
    QString pushGateway() const;
    void setPushGateway(const QString &gateway);
    void setClickableLinks(bool enabled);

    /// Whether the microphone sits next to the message field. On, but it is one
    /// hold away from a recording and not everybody wants that in reach.
    bool voiceMessages() const;
    void setVoiceMessages(bool enabled);

    /// Whether the keyboard goes away once a message is sent. On: the conversation
    /// is what one wants to see afterwards.
    bool hideKeyboardOnSend() const;
    void setHideKeyboardOnSend(bool enabled);

    /// Whether the return key sends. Off: it makes a line break, and the arrow
    /// sends - the only combination in which both are reachable on a touch keyboard.
    bool sendByEnter() const;
    void setSendByEnter(bool enabled);

    /// Whether reactions are drawn from the user's own image files. Off: the
    /// characters cost nothing, an image is a file for a decoder to open.
    bool emojiImages() const;
    void setEmojiImages(bool enabled);

    /// The room directory that is searched, and the list of servers to choose
    /// from beyond the built-in ones.
    QString directoryServer() const;
    void setDirectoryServer(const QString &server);
    QStringList directoryServers() const;
    Q_INVOKABLE void addDirectoryServer(const QString &server);
    Q_INVOKABLE void removeDirectoryServer(const QString &server);

    /// The emoji the picker offers first. Never written shows the built-in
    /// handful; written and empty means the user took them all out.
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
    void jumpToReadMarkerChanged();
    void clickableLinksChanged();
    void pushChanged();
    void voiceMessagesChanged();
    void hideKeyboardOnSendChanged();
    void sendByEnterChanged();
    void emojiImagesChanged();
    void directoryServerChanged();
    void directoryServersChanged();
    void callPolicyChanged();
    void sendReadReceiptsChanged();
    void autoLoadMediaChanged();
    void mediaWipeChanged();
    void emojiFavouritesChanged();
};

#endif // APPSETTINGS_H
