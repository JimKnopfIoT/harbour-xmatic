#include "callengine.h"

#include <QDateTime>
#include <QFile>
#include <QMetaObject>
#include <QUrl>
#include <QVariantMap>

#include <cstring>

#include <gst/gst.h>
#include <gst/sdp/sdp.h>

#define GST_USE_UNSTABLE_API
#include <gst/webrtc/webrtc.h>

#include <gst/video/video.h>

namespace {

/// Audio only. Video is the same negotiation with another media section, but
/// it is not worth adding before a call can be heard.
///
/// The volume element sits in front of the encoder so muting costs nothing and
/// does not disturb the stream: a muted call keeps sending silence rather than
/// stopping, which some clients interpret as a dropped connection.
const char *kAudioPipeline =
    "webrtcbin name=sendrecv bundle-policy=max-bundle latency=100 "
    "autoaudiosrc ! queue leaky=downstream max-size-time=200000000 ! audioconvert ! audioresample ! "
    "volume name=micvolume ! opusenc ! rtpopuspay pt=111 ! "
    "application/x-rtp,media=audio,encoding-name=OPUS,payload=111 ! sendrecv.";

/// The same, plus a camera. VP8 rather than H.264 because the device has no
/// H.264 encoder and Matrix clients negotiate VP8 anyway. The resolution is
/// deliberately modest: every received frame is converted on the CPU before it
/// reaches the screen, since this device has no GStreamer sink for QML.
/// What a call costs must fit the device: a 32-bit build means an old,
/// fanless SoC, so it captures QVGA at 15 frames and half the bitrate. The
/// stronger devices keep VGA at full rate. Everything downstream (including
/// the other side) adapts through the normal negotiation.
// A 32-bit build means an old, fanless SoC that also has to encode this, so
// it sends QVGA at 15 fps; stronger devices keep VGA. (A weak peer receiving
// VGA is a separate, unsolved matter — its WebRTC receive transport errors
// out regardless of resolution, so capping the send does not help it.)
#ifdef Q_PROCESSOR_ARM_32
const int kCallWidth = 320;
const int kCallHeight = 240;
const int kCallFps = 15;
const int kCallBitrate = 256000;
#else
const int kCallWidth = 640;
const int kCallHeight = 480;
const int kCallFps = 30;
const int kCallBitrate = 512000;
#endif

/// Assembled at run time so the size, rate and bitrate above can flow in.
/// The scale caps pin I420: the encoder needs it anyway, and pinning it
/// before the tee means the self-view branch gets it conversion-free too.
QByteArray videoPipelineDescription()
{
    return QStringLiteral(
               "webrtcbin name=sendrecv bundle-policy=max-bundle latency=100 "
               "autoaudiosrc ! queue leaky=downstream max-size-time=200000000 ! audioconvert ! audioresample ! "
               "volume name=micvolume ! opusenc ! rtpopuspay pt=111 ! "
               "application/x-rtp,media=audio,encoding-name=OPUS,payload=111 ! sendrecv. "
               "appsrc name=camsrc is-live=true do-timestamp=true format=time "
               "max-bytes=2000000 ! "
               // max-rate on the element, not a fixed rate in the caps: the
               // camera declares no framerate, and drop-only videorate cannot
               // promise an exact one — that combination refuses to negotiate
               // and the first frame kills the pipeline.
               "videoconvert ! videoscale ! videorate drop-only=true max-rate=%3 ! "
               "video/x-raw,format=I420,width=%1,height=%2 ! "
               // The rotation is set from the device orientation at run time; a
               // fixed one is only correct while the phone is held one way.
               "videoflip name=camflip method=counterclockwise ! "
               // The tee feeds the encoder and the local self-view; it sits
               // after the flip so the preview shows what the other side gets.
               "tee name=selftee ! "
               "queue name=camqueue leaky=downstream max-size-buffers=2 ! "
               "vp8enc deadline=1 threads=4 target-bitrate=%4 keyframe-max-dist=30 ! "
               "rtpvp8pay pt=96 ! "
               "application/x-rtp,media=video,encoding-name=VP8,payload=96 ! sendrecv. "
               // Self-view branch: leaky with a single buffer, because the
               // preview may fall behind but must never stall the encoder.
               "selftee. ! queue leaky=downstream max-size-buffers=1 ! "
               "appsink name=selfsink emit-signals=true max-buffers=2 drop=true")
        .arg(kCallWidth)
        .arg(kCallHeight)
        .arg(kCallFps)
        .arg(kCallBitrate)
        .toLatin1();
}

/// Which decoders a call may plug in.
///
/// Matrix negotiates Opus and VP8, so those are the ones a call needs. Without
/// a list the other side decides which of the phone's decoders parses its
/// bytes - every one of them, gst-libav included, from a stranger's RTP.
/// Also skips gst-droid, which force-sw-decoders does not catch and which
/// takes the call's audio down with it.
gint chooseDecoder(GstElement *, GstPad *, GstCaps *, GstElementFactory *factory, gpointer)
{
    // 2 = GST_AUTOPLUG_SELECT_SKIP, 0 = GST_AUTOPLUG_SELECT_TRY.
    const gchar *name = factory
        ? gst_plugin_feature_get_name(GST_PLUGIN_FEATURE(factory))
        : nullptr;
    if (!name) {
        return 2;
    }
    static const char *const allowed[] = {
        "opusdec", "vp8dec", "vp9dec",
        "rtpopusdepay", "rtpvp8depay", "rtpvp9depay",
        "opusparse", "rtpjitterbuffer",
    };
    for (const char *entry : allowed) {
        if (g_strcmp0(name, entry) == 0) {
            return 0;
        }
    }
    return 2;
}

/// What a remote frame may measure. The sender chooses the size, and the
/// receive path allocates from it.
const int kMaximumRemoteWidth = 1920;
const int kMaximumRemoteHeight = 1920;

/// A random identifier for one call, as the specification requires.
///
/// From the kernel, not from a clock and an LCG: everybody in the room sees a
/// call id, and an id that can be guessed is an id somebody else can answer.
QString freshId(const QString &prefix)
{
    QByteArray random(16, '\0');
    QFile source(QStringLiteral("/dev/urandom"));
    if (source.open(QIODevice::ReadOnly)
        && source.read(random.data(), random.size()) == random.size()) {
        return prefix + QString::fromLatin1(random.toHex());
    }
    // Nothing to fall back to that is worth having; refuse rather than pretend.
    qWarning("xmatic: no random source for a call identifier");
    return QString();
}

} // namespace

CallEngine::CallEngine(QObject *parent)
    : QObject(parent)
{
    m_ringTimeout.setSingleShot(true);
    m_ringTimeout.setInterval(45000);
    connect(&m_ringTimeout, &QTimer::timeout, this, [this]() {
        if (m_state == QLatin1String("ringing")) {
            hangUp();
        }
    });

    // Queued on purpose: frames arrive on the camera's thread, and pushing
    // them from there races with tearing the pipeline down.
    connect(&m_camera,
            &CameraSource::frameReady,
            this,
            &CallEngine::pushCameraFrame,
            Qt::QueuedConnection);
    // A camera that never delivers would leave the offer unbuilt and the
    // other side would never even see the call. Better a voice call that
    // rings than a video call that fails silently.
    m_cameraWatchdog.setSingleShot(true);
    m_cameraWatchdog.setInterval(3000);
    connect(&m_cameraWatchdog, &QTimer::timeout, this, [this]() {
        if (!m_withVideo || m_cameraFrames.load() > 0 || m_state == QLatin1String("idle")) {
            return;
        }
        qWarning("xmatic: the camera produced nothing, falling back to a voice call");
        setStatus(tr("the camera did not start — continuing without video"));

        const QString room = m_roomId;
        hangUp();
        placeCall(room, false);
    });

    m_candidateTimer.setSingleShot(true);
    m_candidateTimer.setInterval(250);
    connect(&m_candidateTimer, &QTimer::timeout, this, &CallEngine::flushCandidates);

    connect(&m_camera, &CameraSource::failed, this, [this](const QString &message) {
        qWarning("xmatic: camera: %s", qPrintable(message));
        setStatus(message);
    });

    GError *error = nullptr;
    if (!gst_init_check(nullptr, nullptr, &error)) {
        m_status = tr("GStreamer could not be started: %1")
                       .arg(error ? QString::fromUtf8(error->message) : QString());
        if (error) {
            g_error_free(error);
        }
        return;
    }

    GstElementFactory *factory = gst_element_factory_find("webrtcbin");
    if (!factory) {
        m_status = tr("This device has no WebRTC support.");
        return;
    }
    gst_object_unref(factory);

    m_available = true;
    m_status = tr("ready");
}

CallEngine::~CallEngine()
{
    tearDown();
}

void CallEngine::setStatus(const QString &status)
{
    if (m_status == status) {
        return;
    }
    m_status = status;
    emit statusChanged();
}

void CallEngine::setState(const QString &state)
{
    if (state != QLatin1String("ringing")) {
        m_ringTimeout.stop();
    }

    if (m_state == state) {
        return;
    }
    m_state = state;
    qInfo("xmatic: call state: %s", qPrintable(state));
    emit callChanged();
}

void CallEngine::setTurnServers(const QVariantList &servers)
{
    m_turnServers = servers;
}

void CallEngine::tearDown()
{
    m_audioRouter.stop();
    m_candidateTimer.stop();
    m_pendingCandidates.clear();
    m_cameraWatchdog.stop();
    m_camera.stop();
    m_cameraFeed = nullptr;
    m_cameraFlip = nullptr;
    if (m_pipeline) {
        gst_element_set_state(m_pipeline, GST_STATE_NULL);
        gst_object_unref(m_pipeline);
        m_pipeline = nullptr;
    }
    m_webrtc = nullptr;
    m_audioSource = nullptr;
}

bool CallEngine::buildPipeline()
{
    tearDown();

    GError *error = nullptr;
    const QByteArray video = videoPipelineDescription();
    m_pipeline = gst_parse_launch(m_withVideo ? video.constData() : kAudioPipeline, &error);
    if (!m_pipeline) {
        const QString message = error ? QString::fromUtf8(error->message) : QString();
        if (error) {
            g_error_free(error);
        }
        reportFailure(tr("pipeline failed: %1").arg(message));
        return false;
    }

    m_webrtc = gst_bin_get_by_name(GST_BIN(m_pipeline), "sendrecv");
    m_audioSource = gst_bin_get_by_name(GST_BIN(m_pipeline), "micvolume");
    if (!m_webrtc) {
        reportFailure(tr("webrtcbin is missing"));
        return false;
    }

    applyTurnServers();

    // Without a bus watch a pipeline failure is completely silent: the call
    // simply never connects and nothing says why.
    GstBus *bus = gst_element_get_bus(m_pipeline);
    // A captureless lambda gives the correctly typed function pointer without
    // moving GStreamer types into the header; casting the member directly
    // would be undefined behaviour.
    gst_bus_add_watch(
        bus,
        [](GstBus *b, GstMessage *m, gpointer u) -> gboolean {
            return CallEngine::busMessage(b, m, u) != 0;
        },
        this);
    gst_object_unref(bus);

    g_signal_connect(m_webrtc, "on-negotiation-needed", G_CALLBACK(negotiationNeeded), this);
    g_signal_connect(m_webrtc, "on-ice-candidate", G_CALLBACK(iceCandidate), this);
    g_signal_connect(m_webrtc, "pad-added", G_CALLBACK(streamAdded), this);
    g_signal_connect(m_webrtc,
                     "notify::connection-state",
                     G_CALLBACK(connectionStateChanged),
                     this);
    g_signal_connect(m_webrtc, "notify::ice-connection-state", G_CALLBACK(iceStateChanged), this);
    g_signal_connect(m_webrtc, "notify::ice-gathering-state", G_CALLBACK(iceStateChanged), this);

    // Our reference is no longer needed; the bin owns the elements.
    gst_object_unref(m_webrtc);
    if (m_audioSource) {
        gst_object_unref(m_audioSource);
    }

    // Count what the camera actually delivers. An outgoing video call that
    // never connects looks identical whether the sensor produced nothing or
    // the negotiation failed, and only this tells the two apart.
    m_cameraFrames.store(0);
    m_cameraFormat.clear();
    m_cameraFeed = m_withVideo ? gst_bin_get_by_name(GST_BIN(m_pipeline), "camsrc") : nullptr;
    m_cameraFlip = m_withVideo ? gst_bin_get_by_name(GST_BIN(m_pipeline), "camflip") : nullptr;
    if (m_cameraFlip) {
        setOrientation(m_orientation);
    }
    if (m_withVideo) {
        GstElement *selfSink = gst_bin_get_by_name(GST_BIN(m_pipeline), "selfsink");
        if (selfSink) {
            g_signal_connect(selfSink, "new-sample", G_CALLBACK(selfFrameArrived), this);
            gst_object_unref(selfSink);
        }
        GstElement *camQueue = gst_bin_get_by_name(GST_BIN(m_pipeline), "camqueue");
        if (camQueue) {
            GstPad *pad = gst_element_get_static_pad(camQueue, "sink");
            if (pad) {
                gst_pad_add_probe(
                    pad,
                    GST_PAD_PROBE_TYPE_BUFFER,
                    [](GstPad *p, GstPadProbeInfo *i, gpointer u) -> GstPadProbeReturn {
                        return static_cast<GstPadProbeReturn>(CallEngine::cameraProbe(p, i, u));
                    },
                    this,
                    nullptr);
                gst_object_unref(pad);
            }
            gst_object_unref(camQueue);
        }
    }

    if (m_muted && m_audioSource) {
        g_object_set(m_audioSource, "mute", TRUE, nullptr);
    }

    return true;
}

void CallEngine::applyTurnServers()
{
    for (const QVariant &entry : m_turnServers) {
        const QVariantMap server = entry.toMap();
        const QString uri = server.value(QStringLiteral("uri")).toString();
        const QString user = server.value(QStringLiteral("username")).toString();
        const QString password = server.value(QStringLiteral("password")).toString();
        if (uri.isEmpty()) {
            continue;
        }

        // webrtcbin wants the credentials inside the URI, percent-encoded, and
        // the scheme kept: `turns:` is the relay over TLS, and rewriting it to
        // `turn:` sent the allocation out in the clear. Only the leading
        // scheme goes, never a match somewhere inside the host.
        QString scheme = QStringLiteral("turn");
        QString address = uri;
        if (address.startsWith(QStringLiteral("turns:"), Qt::CaseInsensitive)) {
            scheme = QStringLiteral("turns");
            address = address.mid(6);
        } else if (address.startsWith(QStringLiteral("turn:"), Qt::CaseInsensitive)) {
            address = address.mid(5);
        }
        const QString full = QStringLiteral("%1://%2:%3@%4")
                                 .arg(scheme,
                                      QString::fromUtf8(QUrl::toPercentEncoding(user)),
                                      QString::fromUtf8(QUrl::toPercentEncoding(password)),
                                      address);

        gboolean added = FALSE;
        g_signal_emit_by_name(m_webrtc, "add-turn-server", full.toUtf8().constData(), &added);
    }
}

void CallEngine::pushCameraFrame(const QByteArray &data,
                                 int width,
                                 int height,
                                 const QString &format)
{
    if (!m_cameraFeed || data.isEmpty()) {
        return;
    }

    // The layout is only known once the first frame arrives, so the caps are
    // set from it rather than assumed.
    if (m_cameraFormat != format) {
        GstCaps *caps = gst_caps_new_simple("video/x-raw",
                                            "format", G_TYPE_STRING, format.toUtf8().constData(),
                                            "width", G_TYPE_INT, width,
                                            "height", G_TYPE_INT, height,
                                            "framerate", GST_TYPE_FRACTION, 15, 1,
                                            nullptr);
        g_object_set(m_cameraFeed, "caps", caps, nullptr);
        gst_caps_unref(caps);
        m_cameraFormat = format;
        qInfo("xmatic: camera format %s, %dx%d", qPrintable(format), width, height);
    }

    GstBuffer *buffer = gst_buffer_new_allocate(nullptr, data.size(), nullptr);
    gst_buffer_fill(buffer, 0, data.constData(), data.size());

    GstFlowReturn flow = GST_FLOW_OK;
    g_signal_emit_by_name(m_cameraFeed, "push-buffer", buffer, &flow);
    gst_buffer_unref(buffer);

    if (flow != GST_FLOW_OK && m_cameraFrames.load() < 2) {
        qWarning("xmatic: camera frame rejected by the pipeline (%d)", int(flow));
    }
}

void CallEngine::placeCall(const QString &roomId, bool withVideo)
{
    if (!m_available || roomId.isEmpty() || m_state != QLatin1String("idle")) {
        return;
    }

    // Without a camera the video branch would never produce caps, the
    // pipeline would not start, and no invitation would go out at all. Falling
    // back to voice is far better than a call that silently never rings.
    m_withVideo = withVideo && CameraSource::isAvailable();
    if (withVideo && !m_withVideo) {
        setStatus(tr("no camera — placing a voice call"));
    }

    m_roomId = roomId;
    m_peer.clear();
    m_callId = freshId(QStringLiteral("c"));
    m_partyId = freshId(QStringLiteral("p"));
    m_isCaller = true;
    m_pendingRemoteOffer.clear();
    emit callChanged();

    setState(QStringLiteral("calling"));
    if (!buildPipeline()) {
        return;
    }
    if (m_withVideo) {
        m_camera.start();
        m_cameraWatchdog.start();
    }

    // Going to PLAYING opens the microphone and triggers negotiation, which
    // produces the offer that ends up in the invitation.
    if (gst_element_set_state(m_pipeline, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE) {
        reportFailure(tr("the microphone could not be opened"));
    }
}

void CallEngine::onRemoteInvite(const QString &roomId,
                                bool videoAllowed,
                                bool videoOffered,
                                const QString &sender,
                                const QString &callId,
                                const QString &sdp)
{
    if (!m_available) {
        return;
    }
    if (m_state != QLatin1String("idle")) {
        // Already busy: the specification wants a hangup rather than silence.
        emit hangupReady(roomId, callId, freshId(QStringLiteral("p")));
        return;
    }

    m_roomId = roomId;
    m_peer = sender;
    m_callId = callId;
    m_partyId = freshId(QStringLiteral("p"));
    m_isCaller = false;
    // What was offered, not what will be answered: the camera opens when the
    // user picks the action that opens it, never because the offer asked. The
    // core has already applied the privacy setting to this flag - with video
    // calls switched off it arrives false and the two actions collapse to one.
    m_videoOffered = videoAllowed && CameraSource::isAvailable();
    m_videoRefused = videoOffered && !m_videoOffered;
    m_withVideo = false;
    m_pendingRemoteOffer = sdp;
    emit callChanged();

    setState(QStringLiteral("ringing"));
    // A ring that nobody answers ends by itself. Without this the ringtone
    // loops until the caller gives up - and a caller can choose not to.
    m_ringTimeout.start();
    emit incomingCall(roomId, sender);
}

void CallEngine::setExpectedPeer(const QString &peer)
{
    if (!m_isCaller || peer.isEmpty() || m_state == QLatin1String("idle")) {
        return;
    }
    m_peer = peer;
    emit callChanged();
}

void CallEngine::acceptCall(bool withVideo)
{
    m_withVideo = withVideo && m_videoOffered;
    if (m_state != QLatin1String("ringing") || m_pendingRemoteOffer.isEmpty()) {
        return;
    }

    setState(QStringLiteral("connecting"));
    if (!buildPipeline()) {
        return;
    }
    if (m_withVideo) {
        m_camera.start();
    }

    if (gst_element_set_state(m_pipeline, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE) {
        reportFailure(tr("the microphone could not be opened"));
        return;
    }

    // Answering means taking their offer first; the answer follows from it.
    applyRemoteDescription(m_pendingRemoteOffer, true);
    m_pendingRemoteOffer.clear();
}

void CallEngine::onRemoteAnswer(const QString &roomId,
                               const QString &sender,
                               const QString &callId,
                               const QString &sdp)
{
    // The id alone is not an authentication: in a room with more than two
    // members everybody sees it. Who we called is not known in advance, so the
    // first answer decides it and every later event has to come from the same
    // party - that is what MSC2746 calls selecting an answer.
    if (callId != m_callId || !m_isCaller || roomId != m_roomId) {
        return;
    }
    if (!m_peer.isEmpty() && sender != m_peer) {
        return;
    }
    m_peer = sender;
    setState(QStringLiteral("connecting"));
    applyRemoteDescription(sdp, false);
}

void CallEngine::applyRemoteDescription(const QString &sdp, bool isOffer)
{
    if (!m_webrtc) {
        return;
    }

    GstSDPMessage *message = nullptr;
    if (gst_sdp_message_new(&message) != GST_SDP_OK) {
        reportFailure(tr("the session description could not be read"));
        return;
    }

    const QByteArray text = sdp.toUtf8();
    if (gst_sdp_message_parse_buffer(reinterpret_cast<const guint8 *>(text.constData()),
                                     text.size(),
                                     message)
        != GST_SDP_OK) {
        gst_sdp_message_free(message);
        reportFailure(tr("the other side sent an unreadable session description"));
        return;
    }

    GstWebRTCSessionDescription *description = gst_webrtc_session_description_new(
        isOffer ? GST_WEBRTC_SDP_TYPE_OFFER : GST_WEBRTC_SDP_TYPE_ANSWER,
        message);

    GstPromise *promise = gst_promise_new();
    g_signal_emit_by_name(m_webrtc, "set-remote-description", description, promise);
    gst_promise_interrupt(promise);
    gst_promise_unref(promise);
    gst_webrtc_session_description_free(description);

    if (isOffer) {
        // Now that their offer is known, ask for our answer.
        GstPromise *answer = gst_promise_new_with_change_func(descriptionCreated, this, nullptr);
        g_signal_emit_by_name(m_webrtc, "create-answer", nullptr, answer);
    }
}

void CallEngine::onRemoteCandidates(const QString &roomId,
                                   const QString &sender,
                                   const QString &callId,
                                   const QVariantList &candidates)
{
    // Same rule as the answer: an injected candidate makes this device send
    // STUN probes to an address of somebody else's choosing. Before the party
    // is known - our own call, not yet answered - the room is the whole check.
    if (callId != m_callId || !m_webrtc || roomId != m_roomId) {
        return;
    }
    if (!m_peer.isEmpty() && sender != m_peer) {
        return;
    }

    for (const QVariant &entry : candidates) {
        const QVariantMap candidate = entry.toMap();
        const QString value = candidate.value(QStringLiteral("candidate")).toString();
        if (value.isEmpty()) {
            // The end-of-candidates marker; nothing to add.
            continue;
        }
        const unsigned index = candidate.value(QStringLiteral("sdpMLineIndex")).toUInt();
        g_signal_emit_by_name(m_webrtc, "add-ice-candidate", index, value.toUtf8().constData());
    }
}

void CallEngine::onRemoteHangup(const QString &roomId,
                               const QString &sender,
                               const QString &callId)
{
    // Room and party, as for the answer: the call id is public to every member
    // of the room, so on its own it lets any bystander end the call.
    if (!roomId.isEmpty() && roomId != m_roomId) {
        return;
    }
    if (!m_peer.isEmpty() && !sender.isEmpty() && sender != m_peer) {
        return;
    }

    if (m_state == QLatin1String("idle")) {
        return;
    }
    // Match strictly: only a hangup naming this exact call ends it. The old
    // loose match — honouring a different or empty call id "to be safe" —
    // let stale hangup events, of which a busy test room accumulates many,
    // replay on sync and kill a freshly placed call within a second. A
    // genuinely dropped connection is caught by the connection-state teardown,
    // and the user's own hang-up button by hangUp(); neither needs this to be
    // lenient.
    if (callId.isEmpty() || m_callId.isEmpty() || callId != m_callId) {
        qInfo("xmatic: ignoring hangup for another call");
        return;
    }
    setStatus(tr("the other side hung up"));
    tearDown();
    m_remoteVideo.stop();
    m_withVideo = false;
    m_callId.clear();
    m_roomId.clear();
    m_peer.clear();
    setState(QStringLiteral("idle"));
    emit callChanged();
}

void CallEngine::hangUp()
{
    if (m_state == QLatin1String("idle")) {
        return;
    }

    if (!m_roomId.isEmpty() && !m_callId.isEmpty()) {
        emit hangupReady(m_roomId, m_callId, m_partyId);
    }

    tearDown();
    m_callId.clear();
    m_roomId.clear();
    m_peer.clear();
    m_pendingRemoteOffer.clear();
    m_withVideo = false;
    m_remoteVideo.stop();
    setState(QStringLiteral("idle"));
    emit callChanged();
}

void CallEngine::setOrientation(int orientation)
{
    m_orientation = orientation;
    if (!m_cameraFlip) {
        return;
    }

    // GstVideoFlipMethod: 0 none, 1 a quarter turn clockwise, 2 half a turn,
    // 3 a quarter turn anticlockwise. The sensor sits a quarter turn from the
    // upright phone, so every orientation shifts that baseline.
    int method = 3;
    switch (orientation) {
    case 1: // Orientation.Portrait
        method = 3;
        break;
    case 2: // Orientation.Landscape
        method = 0;
        break;
    case 4: // Orientation.PortraitInverted
        method = 1;
        break;
    case 8: // Orientation.LandscapeInverted
        method = 2;
        break;
    default:
        method = 3;
        break;
    }

    g_object_set(m_cameraFlip, "method", method, nullptr);
    qInfo("xmatic: camera rotation method %d for orientation %d", method, orientation);
}

void CallEngine::setMuted(bool muted)
{
    if (m_muted == muted) {
        return;
    }
    m_muted = muted;
    if (m_audioSource) {
        g_object_set(m_audioSource, "mute", muted ? TRUE : FALSE, nullptr);
    }
    emit mutedChanged();
}

void CallEngine::negotiationNeeded(GstElement *webrtc, void *user)
{
    auto *engine = static_cast<CallEngine *>(user);
    if (!engine->m_isCaller) {
        // The answering side negotiates from the remote offer, not on its own.
        return;
    }
    GstPromise *promise = gst_promise_new_with_change_func(descriptionCreated, user, nullptr);
    g_signal_emit_by_name(webrtc, "create-offer", nullptr, promise);
}

void CallEngine::descriptionCreated(GstPromise *promise, void *user)
{
    auto *engine = static_cast<CallEngine *>(user);

    const GstStructure *reply = gst_promise_get_reply(promise);
    if (!reply) {
        // An interrupted promise (pipeline torn down mid-negotiation) has no
        // reply structure.
        gst_promise_unref(promise);
        return;
    }
    GstWebRTCSessionDescription *description = nullptr;
    const bool isOffer = gst_structure_has_field(reply, "offer");
    gst_structure_get(reply,
                      isOffer ? "offer" : "answer",
                      GST_TYPE_WEBRTC_SESSION_DESCRIPTION,
                      &description,
                      nullptr);
    gst_promise_unref(promise);

    if (!description) {
        QMetaObject::invokeMethod(engine,
                                  "reportFailure",
                                  Qt::QueuedConnection,
                                  Q_ARG(QString, QObject::tr("no session description")));
        return;
    }

    // Ours has to be applied locally before it is sent, or the candidates
    // gathered afterwards belong to nothing.
    GstPromise *applied = gst_promise_new();
    g_signal_emit_by_name(engine->m_webrtc, "set-local-description", description, applied);
    gst_promise_interrupt(applied);
    gst_promise_unref(applied);

    gchar *text = gst_sdp_message_as_text(description->sdp);
    const QString sdp = QString::fromUtf8(text);
    g_free(text);
    gst_webrtc_session_description_free(description);

    QMetaObject::invokeMethod(engine,
                              "deliverLocalDescription",
                              Qt::QueuedConnection,
                              Q_ARG(QString, sdp),
                              Q_ARG(bool, isOffer));
}

void CallEngine::iceCandidate(GstElement *, unsigned index, char *candidate, void *user)
{
    auto *engine = static_cast<CallEngine *>(user);
    QMetaObject::invokeMethod(engine,
                              "deliverCandidate",
                              Qt::QueuedConnection,
                              Q_ARG(QString, QString::fromUtf8(candidate)),
                              Q_ARG(int, static_cast<int>(index)));
}

void CallEngine::streamAdded(GstElement *, GstPad *pad, void *user)
{
    auto *engine = static_cast<CallEngine *>(user);
    if (!engine->m_pipeline || GST_PAD_DIRECTION(pad) != GST_PAD_SRC) {
        return;
    }

    // The incoming stream arrives as RTP. decodebin works out the rest and
    // announces a pad once it knows whether this is audio or video, which is
    // where the two branches part.
    GstElement *decode = gst_element_factory_make("decodebin", nullptr);
    if (!decode) {
        return;
    }
    // Skip the Android hardware decoder for the incoming VP8. On the Gemini
    // decodebin otherwise plugs droidvdec, which refuses the caps
    // ("not-negotiated") and kills the whole receive transport — taking the
    // call's audio down with the video, since they share it. force-sw-decoders
    // alone does not skip it (gst-droid does not tag it as hardware), so reject
    // any droid* factory explicitly. VP8 in software is cheap enough.
    g_object_set(decode, "force-sw-decoders", TRUE, nullptr);
    g_signal_connect(decode, "autoplug-select", G_CALLBACK(chooseDecoder), nullptr);

    gst_bin_add(GST_BIN(engine->m_pipeline), decode);
    // Connect the handler and link the incoming pad BEFORE decodebin starts,
    // then sync it to PLAYING last — same not-linked race as the branches.
    g_signal_connect(decode, "pad-added", G_CALLBACK(decodedPadAdded), engine);

    GstPad *decodeSink = gst_element_get_static_pad(decode, "sink");
    if (decodeSink) {
        gst_pad_link(pad, decodeSink);
        gst_object_unref(decodeSink);
    }

    gst_element_sync_state_with_parent(decode);
}

void CallEngine::decodedPadAdded(GstElement *, GstPad *pad, void *user)
{
    auto *engine = static_cast<CallEngine *>(user);
    if (!engine->m_pipeline) {
        return;
    }

    GstCaps *caps = gst_pad_get_current_caps(pad);
    if (!caps) {
        return;
    }
    const GstStructure *structure = gst_caps_get_structure(caps, 0);
    const gchar *name = gst_structure_get_name(structure);
    const bool isVideo = name && g_str_has_prefix(name, "video/");
    gst_caps_unref(caps);

    // A call answered without video decodes no video. The privacy switch shut
    // the camera; it has to shut the decoder too, or the other side still
    // chooses which of this phone's decoders runs on its bytes.
    if (isVideo && !engine->m_withVideo) {
        return;
    }

    GstElement *first = nullptr;
    GstElement *last = nullptr;
    // The branch elements, kept so they can be started only AFTER everything
    // is linked. Syncing them to the pipeline's PLAYING state before the pad
    // is linked lets the decoder push into an unlinked pad — GST_FLOW_NOT_LINKED
    // — which surfaces as "Internal data stream error" from the receive
    // transport and kills the whole call.
    GstElement *chain[4] = {nullptr, nullptr, nullptr, nullptr};
    int chainN = 0;

    if (isVideo) {
        // Straight to RGB: the frames are handed to Qt as images, so the
        // conversion has to happen here rather than on the QML side.
        // A leaky queue decouples video decoding from the shared transport
        // thread: without it a slow decoder (the Gemini decoding an incoming
        // VGA stream) stalls the whole receive path and takes the audio with
        // it. Dropping late video is the right trade — audio must keep flowing.
        GstElement *queue = gst_element_factory_make("queue", nullptr);
        GstElement *convert = gst_element_factory_make("videoconvert", nullptr);
        GstElement *sink = gst_element_factory_make("appsink", nullptr);
        if (!queue || !convert || !sink) {
            // Whatever was created has to be released by hand: the elements are
            // only handed to the bin further down, so leaving here would drop
            // the last reference to them without anyone taking ownership.
            // Missing plugins are the only way in, so this is a small and
            // deterministic leak — but it sits in the path that exists to
            // survive exactly that.
            if (queue) {
                gst_object_unref(queue);
            }
            if (convert) {
                gst_object_unref(convert);
            }
            if (sink) {
                gst_object_unref(sink);
            }
            return;
        }
        g_object_set(queue, "leaky", 2, "max-size-buffers", 3, nullptr);

        // I420 is what the decoder produces, so the convert element in front
        // is a passthrough and the colour conversion happens on the GPU when
        // the frame is drawn.
        // No size in these caps. They sit downstream of the decoder, so they
        // cannot shrink what it already allocated - all they would do is turn
        // an oversize frame into a not-negotiated error, and that error kills
        // the shared transport, taking the call's audio with it. The frame is
        // bounded where it is copied instead.
        GstCaps *wanted = gst_caps_new_simple("video/x-raw",
                                              "format", G_TYPE_STRING, "I420",
                                              nullptr);
        g_object_set(sink, "caps", wanted, "emit-signals", TRUE, "max-buffers", 2,
                     "drop", TRUE, nullptr);
        gst_caps_unref(wanted);

        g_signal_connect(sink, "new-sample", G_CALLBACK(videoFrameArrived), engine);

        gst_bin_add_many(GST_BIN(engine->m_pipeline), queue, convert, sink, nullptr);
        gst_element_link_many(queue, convert, sink, nullptr);

        first = queue;
        last = sink;
        chain[chainN++] = queue;
        chain[chainN++] = convert;
        chain[chainN++] = sink;
    } else {
        // The queue is what keeps audio playing while video decodes on the
        // same pipeline: it gives the audio branch its own thread so a busy
        // video path can no longer starve it (silent audio in video calls,
        // while voice-only calls were fine).
        GstElement *queue = gst_element_factory_make("queue", nullptr);
        GstElement *convert = gst_element_factory_make("audioconvert", nullptr);
        GstElement *resample = gst_element_factory_make("audioresample", nullptr);
        // pulsesink, not autoaudiosink: the latter follows PulseAudio's
        // default sink, which on these devices is sink.null — so received
        // audio played into nothing. A bare pulsesink routes to the real
        // output through the droid card policy, on Xperia and Gemini alike.
        GstElement *sink = gst_element_factory_make("pulsesink", nullptr);
        // And bypass module-stream-restore: it had the default "x-maemo"
        // stream role remembered as muted, so every received call stream
        // inherited that mute and the call was silent even though audio
        // reached PulseAudio. An empty restore id opts this stream out of the
        // saved per-role state; mute=false sets it explicitly regardless.
        if (sink) {
            GstStructure *props = gst_structure_new("stream-properties",
                                                    "application.name",
                                                    G_TYPE_STRING,
                                                    "xmatic call",
                                                    nullptr);
            g_object_set(sink, "stream-properties", props, nullptr);
            gst_structure_free(props);
        }
        if (!queue || !convert || !resample || !sink) {
            return;
        }
        gst_bin_add_many(GST_BIN(engine->m_pipeline), queue, convert, resample, sink, nullptr);
        gst_element_link_many(queue, convert, resample, sink, nullptr);

        first = queue;
        last = sink;
        chain[chainN++] = queue;
        chain[chainN++] = convert;
        chain[chainN++] = resample;
        chain[chainN++] = sink;
    }

    Q_UNUSED(last)

    GstPad *sinkPad = gst_element_get_static_pad(first, "sink");
    if (sinkPad) {
        if (!gst_pad_is_linked(sinkPad)) {
            gst_pad_link(pad, sinkPad);
        }
        // Everything is linked now — only here is it safe to start the branch,
        // so the decoder never pushes into an unlinked pad.
        for (int i = 0; i < chainN; ++i) {
            gst_element_sync_state_with_parent(chain[i]);
        }
        gst_object_unref(sinkPad);
    }
}

int CallEngine::videoFrameArrived(void *sink, void *user)
{
    auto *engine = static_cast<CallEngine *>(user);

    GstSample *sample = nullptr;
    g_signal_emit_by_name(static_cast<GstElement *>(sink), "pull-sample", &sample);
    if (!sample) {
        return 0;
    }

    GstCaps *caps = gst_sample_get_caps(sample);
    GstBuffer *buffer = gst_sample_get_buffer(sample);

    GstVideoInfo info;
    if (caps && buffer && gst_video_info_from_caps(&info, caps)
        && GST_VIDEO_INFO_FORMAT(&info) == GST_VIDEO_FORMAT_I420) {
        GstVideoFrame frame;
        if (gst_video_frame_map(&frame, &info, buffer, GST_MAP_READ)) {
            const int width = GST_VIDEO_FRAME_WIDTH(&frame);
            const int height = GST_VIDEO_FRAME_HEIGHT(&frame);
            // What the other side sends decides this allocation, so it is
            // bounded here. Beyond the cap the frame is dropped rather than
            // copied; a call that sends 4K to a phone is not a call.
            if (width <= 0 || height <= 0 || width > kMaximumRemoteWidth
                || height > kMaximumRemoteHeight) {
                gst_video_frame_unmap(&frame);
                gst_sample_unref(sample);
                return GST_FLOW_OK;
            }
            // Repack into tight I420: the decoder's planes may be padded to an
            // alignment (16 or 32 on the Gemini), and a QVideoFrame wants
            // contiguous rows. Copy row by row from each plane's real stride.
            QByteArray tight;
            tight.resize(width * height * 3 / 2);
            char *out = tight.data();
            for (int plane = 0; plane < 3; ++plane) {
                const int pw = plane == 0 ? width : width / 2;
                const int ph = plane == 0 ? height : height / 2;
                const int stride = GST_VIDEO_FRAME_PLANE_STRIDE(&frame, plane);
                const char *in =
                    static_cast<const char *>(GST_VIDEO_FRAME_PLANE_DATA(&frame, plane));
                for (int row = 0; row < ph; ++row) {
                    memcpy(out, in + row * stride, pw);
                    out += pw;
                }
            }
            gst_video_frame_unmap(&frame);
            QMetaObject::invokeMethod(engine,
                                      "deliverVideoYuv",
                                      Qt::QueuedConnection,
                                      Q_ARG(QByteArray, tight),
                                      Q_ARG(int, width),
                                      Q_ARG(int, height));
        }
    }

    gst_sample_unref(sample);
    return 0;
}

void CallEngine::deliverVideoFrame(const QImage &frame)
{
    m_remoteVideo.present(frame);
}

void CallEngine::deliverVideoYuv(const QByteArray &planes, int width, int height)
{
    m_remoteVideo.presentYuv(planes, QSize(width, height));
}

void CallEngine::deliverSelfYuv(const QByteArray &planes, int width, int height)
{
    m_selfVideo.presentYuv(planes, QSize(width, height));
}

int CallEngine::selfFrameArrived(void *sink, void *user)
{
    auto *engine = static_cast<CallEngine *>(user);

    GstSample *sample = nullptr;
    g_signal_emit_by_name(static_cast<GstElement *>(sink), "pull-sample", &sample);
    if (!sample) {
        return 0;
    }

    GstCaps *caps = gst_sample_get_caps(sample);
    GstBuffer *buffer = gst_sample_get_buffer(sample);

    GstVideoInfo info;
    if (caps && buffer && gst_video_info_from_caps(&info, caps)
        && GST_VIDEO_INFO_FORMAT(&info) == GST_VIDEO_FORMAT_I420) {
        GstVideoFrame frame;
        if (gst_video_frame_map(&frame, &info, buffer, GST_MAP_READ)) {
            const int width = GST_VIDEO_FRAME_WIDTH(&frame);
            const int height = GST_VIDEO_FRAME_HEIGHT(&frame);
            if (width <= 0 || height <= 0 || width > kMaximumRemoteWidth
                || height > kMaximumRemoteHeight) {
                gst_video_frame_unmap(&frame);
                gst_sample_unref(sample);
                return GST_FLOW_OK;
            }
            QByteArray tight;
            tight.resize(width * height * 3 / 2);
            char *out = tight.data();
            for (int plane = 0; plane < 3; ++plane) {
                const int pw = plane == 0 ? width : width / 2;
                const int ph = plane == 0 ? height : height / 2;
                const int stride = GST_VIDEO_FRAME_PLANE_STRIDE(&frame, plane);
                const char *in =
                    static_cast<const char *>(GST_VIDEO_FRAME_PLANE_DATA(&frame, plane));
                for (int row = 0; row < ph; ++row) {
                    memcpy(out, in + row * stride, pw);
                    out += pw;
                }
            }
            gst_video_frame_unmap(&frame);
            QMetaObject::invokeMethod(engine,
                                      "deliverSelfYuv",
                                      Qt::QueuedConnection,
                                      Q_ARG(QByteArray, tight),
                                      Q_ARG(int, width),
                                      Q_ARG(int, height));
        }
    }

    gst_sample_unref(sample);
    return 0;
}

void CallEngine::deliverSelfFrame(const QImage &frame)
{
    m_selfVideo.present(frame);
}

void CallEngine::connectionStateChanged(GstElement *webrtc, void *, void *user)
{
    GstWebRTCPeerConnectionState state = GST_WEBRTC_PEER_CONNECTION_STATE_NEW;
    g_object_get(webrtc, "connection-state", &state, nullptr);

    auto *engine = static_cast<CallEngine *>(user);

    if (state == GST_WEBRTC_PEER_CONNECTION_STATE_CONNECTED) {
        QMetaObject::invokeMethod(engine, "markConnected", Qt::QueuedConnection);
        return;
    }

    // A connection that fails or closes has to return the engine to idle by
    // itself. Otherwise the call never ends from this side's point of view and
    // every later call is silently refused as "busy".
    if (state == GST_WEBRTC_PEER_CONNECTION_STATE_FAILED
        || state == GST_WEBRTC_PEER_CONNECTION_STATE_CLOSED) {
        QMetaObject::invokeMethod(engine, "markDisconnected", Qt::QueuedConnection);
    }
}

unsigned CallEngine::cameraProbe(GstPad *, void *, void *user)
{
    auto *engine = static_cast<CallEngine *>(user);

    // Only the first few are worth reporting; after that the camera clearly
    // works and the log would drown.
    const int seen = engine->m_cameraFrames.fetchAndAddOrdered(1) + 1;
    if (seen == 1 || seen == 30) {
        qInfo("xmatic: camera delivered %u frames", seen);
    }

    return GST_PAD_PROBE_OK;
}

int CallEngine::busMessage(void *, void *message, void *)
{
    auto *msg = static_cast<GstMessage *>(message);

    switch (GST_MESSAGE_TYPE(msg)) {
    case GST_MESSAGE_ERROR: {
        GError *error = nullptr;
        gchar *debug = nullptr;
        gst_message_parse_error(msg, &error, &debug);
        qWarning("xmatic: pipeline error from %s: %s",
                 GST_OBJECT_NAME(msg->src),
                 error ? error->message : "?");
        if (debug) {
            qWarning("xmatic: pipeline detail: %s", debug);
            g_free(debug);
        }
        if (error) {
            g_error_free(error);
        }
        break;
    }
    case GST_MESSAGE_WARNING: {
        GError *error = nullptr;
        gst_message_parse_warning(msg, &error, nullptr);
        qWarning("xmatic: pipeline warning from %s: %s",
                 GST_OBJECT_NAME(msg->src),
                 error ? error->message : "?");
        if (error) {
            g_error_free(error);
        }
        break;
    }
    default:
        break;
    }

    return TRUE;
}

void CallEngine::iceStateChanged(GstElement *webrtc, void *, void *)
{
    GstWebRTCICEConnectionState connection = GST_WEBRTC_ICE_CONNECTION_STATE_NEW;
    GstWebRTCICEGatheringState gathering = GST_WEBRTC_ICE_GATHERING_STATE_NEW;
    g_object_get(webrtc,
                 "ice-connection-state",
                 &connection,
                 "ice-gathering-state",
                 &gathering,
                 nullptr);

    // Numbers rather than names: the enums are stable and this is a trace, not
    // a message for the user.
    qInfo("xmatic: ICE connection=%d gathering=%d", int(connection), int(gathering));
}

void CallEngine::markDisconnected()
{
    if (m_state == QLatin1String("idle")) {
        return;
    }
    qInfo("xmatic: call connection lost");
    setStatus(tr("the connection was lost"));
    hangUp();
}

void CallEngine::markConnected()
{
    if (m_withVideo && m_cameraFrames.load() == 0) {
        qWarning("xmatic: connected but the camera has produced nothing");
    }

    m_audioRouter.start();
    setState(QStringLiteral("active"));
    setStatus(tr("connected"));
}

void CallEngine::deliverLocalDescription(const QString &sdp, bool isOffer)
{
    qInfo("xmatic: local %s ready, %d bytes", isOffer ? "offer" : "answer", sdp.size());

    if (isOffer) {
        emit inviteReady(m_roomId, m_callId, m_partyId, sdp);
    } else {
        emit answerReady(m_roomId, m_callId, m_partyId, sdp);
    }
}

void CallEngine::deliverCandidate(const QString &candidate, int mediaLineIndex)
{
    if (m_roomId.isEmpty() || m_callId.isEmpty()) {
        return;
    }

    QVariantMap entry;
    entry.insert(QStringLiteral("candidate"), candidate);
    entry.insert(QStringLiteral("sdpMLineIndex"), mediaLineIndex);

    // Batched: ICE produces bursts of candidates, and one room event per
    // candidate runs straight into the homeserver's rate limit — the server
    // then rejects the batch's tail with 429 and the call never connects.
    // A quarter second of collecting turns a dozen events into one.
    m_pendingCandidates.append(entry);
    if (!m_candidateTimer.isActive()) {
        m_candidateTimer.start();
    }
}

void CallEngine::flushCandidates()
{
    if (m_pendingCandidates.isEmpty() || m_roomId.isEmpty() || m_callId.isEmpty()) {
        m_pendingCandidates.clear();
        return;
    }
    const QVariantList batch = m_pendingCandidates;
    m_pendingCandidates.clear();
    emit candidatesReady(m_roomId, m_callId, m_partyId, batch);
}

void CallEngine::reportFailure(const QString &message)
{
    qWarning("xmatic: call engine: %s", qPrintable(message));
    setStatus(message);
    tearDown();
    setState(QStringLiteral("idle"));
}
