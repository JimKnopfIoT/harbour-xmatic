#ifndef CALLENGINE_H
#define CALLENGINE_H

#include <QAtomicInt>
#include <QObject>
#include <QByteArray>
#include <QTimer>
#include <QImage>
#include <QString>
#include <QVariantList>

#include "camerasource.h"
#include "callaudiorouter.h"
#include "videostream.h"

typedef struct _GstElement GstElement;
typedef struct _GstPromise GstPromise;
typedef struct _GstPad GstPad;

/// The media half of a call.
///
/// Matrix carries only the negotiation — who rings, who answers, and the
/// session descriptions and candidates. The audio itself travels directly
/// between the two devices over WebRTC, which matrix-rust-sdk does not
/// implement, so this drives GStreamer's `webrtcbin` with libnice, DTLS-SRTP
/// and Opus.
///
/// This object owns no signalling: it emits what has to be sent and is fed
/// what arrives. The bridge connects both ends to the core.
class CallEngine : public QObject
{
    Q_OBJECT

    Q_PROPERTY(bool available READ available CONSTANT)
    Q_PROPERTY(QString status READ status NOTIFY statusChanged)

    /// One of "idle", "calling", "ringing", "connecting", "active".
    Q_PROPERTY(QString state READ state NOTIFY callChanged)
    Q_PROPERTY(QString roomId READ roomId NOTIFY callChanged)
    Q_PROPERTY(QString peer READ peer NOTIFY callChanged)
    Q_PROPERTY(bool muted READ muted NOTIFY mutedChanged)
    Q_PROPERTY(bool video READ video NOTIFY callChanged)
    Q_PROPERTY(bool videoOffered READ videoOffered NOTIFY callChanged)
    Q_PROPERTY(bool videoRefused READ videoRefused NOTIFY callChanged)
    Q_PROPERTY(QObject *remoteVideo READ remoteVideo CONSTANT)
    Q_PROPERTY(QObject *selfVideo READ selfVideo CONSTANT)

public:
    explicit CallEngine(QObject *parent = nullptr);
    ~CallEngine() override;

    bool available() const { return m_available; }
    QString status() const { return m_status; }
    QString state() const { return m_state; }
    QString roomId() const { return m_roomId; }
    QString peer() const { return m_peer; }
    bool muted() const { return m_muted; }
    bool video() const { return m_withVideo; }
    /// Whether the ringing call offered video. Says what the two accept
    /// actions mean; it does not by itself open anything.
    bool videoOffered() const { return m_videoOffered; }
    /// The caller offered video and the privacy setting says no. The call goes
    /// through as a voice call, and the page says why.
    bool videoRefused() const { return m_videoRefused; }
    QObject *remoteVideo() { return &m_remoteVideo; }
    QObject *selfVideo() { return &m_selfVideo; }

    /// Rings the other side of `roomId`. With `withVideo` the offer carries a
    /// camera stream as well.
    Q_INVOKABLE void placeCall(const QString &roomId, bool withVideo = false);

    /// Answers the call that is currently ringing.
    /// Answers the ringing call. `withVideo` opens the camera - never the
    /// caller's choice, always the user's, and only where the offer had video
    /// at all.
    Q_INVOKABLE void acceptCall(bool withVideo = false);

    /// Who may answer the call this device just placed. Set from the reply to
    /// the invitation, before any answer can arrive: without it the first
    /// answer from anybody in the room would take the microphone.
    void setExpectedPeer(const QString &peer);

    /// Ends or declines the current call.
    Q_INVOKABLE void hangUp();

    /// Silences the microphone without ending the call.
    Q_INVOKABLE void setMuted(bool muted);

    /// Tells the engine how the phone is being held, so the picture it sends
    /// stays upright. Takes a Silica Orientation value.
    Q_INVOKABLE void setOrientation(int orientation);

    /// Relay credentials, applied to the next call. Without them two devices
    /// behind NAT connect and stay silent.
    void setTurnServers(const QVariantList &servers);

    // Fed from the core's signalling events.
    void onRemoteInvite(const QString &roomId,
                        bool videoAllowed,
                        bool videoOffered,
                        const QString &sender,
                        const QString &callId,
                        const QString &sdp);
    void onRemoteAnswer(const QString &roomId,
                        const QString &sender,
                        const QString &callId,
                        const QString &sdp);
    void onRemoteCandidates(const QString &roomId,
                            const QString &sender,
                            const QString &callId,
                            const QVariantList &candidates);
    void onRemoteHangup(const QString &roomId, const QString &sender, const QString &callId);

signals:
    void statusChanged();
    void callChanged();
    void mutedChanged();

    // What the core has to put on the wire.
    void inviteReady(const QString &roomId,
                     const QString &callId,
                     const QString &partyId,
                     const QString &sdp);
    void answerReady(const QString &roomId,
                     const QString &callId,
                     const QString &partyId,
                     const QString &sdp);
    void candidatesReady(const QString &roomId,
                         const QString &callId,
                         const QString &partyId,
                         const QVariantList &candidates);
    void hangupReady(const QString &roomId, const QString &callId, const QString &partyId);

    /// Someone is calling; the UI should show the incoming call screen.
    void incomingCall(const QString &roomId, const QString &peer);

private slots:
    void deliverLocalDescription(const QString &sdp, bool isOffer);
    void deliverVideoFrame(const QImage &frame);
    void deliverSelfFrame(const QImage &frame);
    void deliverVideoYuv(const QByteArray &planes, int width, int height);
    void deliverSelfYuv(const QByteArray &planes, int width, int height);
    void flushCandidates();
    void pushCameraFrame(const QByteArray &data, int width, int height, const QString &format);
    void deliverCandidate(const QString &candidate, int mediaLineIndex);
    void reportFailure(const QString &message);
    void markConnected();
    void markDisconnected();

private:
    static void negotiationNeeded(GstElement *webrtc, void *user);
    static void descriptionCreated(GstPromise *promise, void *user);
    static void iceCandidate(GstElement *webrtc, unsigned index, char *candidate, void *user);
    static void streamAdded(GstElement *webrtc, GstPad *pad, void *user);
    static void decodedPadAdded(GstElement *decode, GstPad *pad, void *user);
    static int busMessage(void *bus, void *message, void *user);
    static unsigned cameraProbe(GstPad *pad, void *info, void *user);
    static void iceStateChanged(GstElement *webrtc, void *spec, void *user);
    static int videoFrameArrived(void *sink, void *user);
    static int selfFrameArrived(void *sink, void *user);
    static void connectionStateChanged(GstElement *webrtc, void *spec, void *user);

    bool buildPipeline();
    void applyTurnServers();
    void applyRemoteDescription(const QString &sdp, bool isOffer);
    void tearDown();
    void setStatus(const QString &status);
    void setState(const QString &state);

    GstElement *m_pipeline = nullptr;
    GstElement *m_webrtc = nullptr;
    GstElement *m_audioSource = nullptr;

    bool m_available = false;
    bool m_muted = false;
    /// True while this side is the caller, which decides offer versus answer.
    bool m_isCaller = false;
    /// Whether this call negotiated a camera stream.
    bool m_withVideo = false;
    bool m_videoOffered = false;
    /// Stops a ring nobody answers.
    QTimer m_ringTimeout;
    bool m_videoRefused = false;

    QString m_status;
    QString m_state = QStringLiteral("idle");
    QString m_roomId;
    QString m_peer;
    QString m_callId;
    QString m_partyId;
    QString m_pendingRemoteOffer;
    QVariantList m_turnServers;
    VideoStream m_remoteVideo;
    VideoStream m_selfVideo;
    CallAudioRouter m_audioRouter;
    QVariantList m_pendingCandidates;
    QTimer m_candidateTimer;
    CameraSource m_camera;
    /// Falls back to voice when the camera stays silent.
    QTimer m_cameraWatchdog;
    GstElement *m_cameraFeed = nullptr;
    GstElement *m_cameraFlip = nullptr;
    int m_orientation = 1;
    QString m_cameraFormat;

    /// Frames the camera has delivered in this call; zero means it never ran.
    // Written on the camera's streaming thread, read on the Qt one: the
    // watchdog that reads a stale zero tears a working video call down.
    QAtomicInt m_cameraFrames;
};

#endif // CALLENGINE_H
