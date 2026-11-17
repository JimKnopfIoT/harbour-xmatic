#include "callengine.h"

#include <QTimer>
#include <QTransform>
#include <QVector>

#include <QDateTime>
#include <QFile>
#include <QMetaObject>
#include <QMutex>
#include <QMutexLocker>
#include <QUrl>
#include <QVariantMap>

#include <atomic>
#include <climits>
#include <cstring>
#include <utility>

#include <gst/gst.h>
#include <gst/sdp/sdp.h>

#define GST_USE_UNSTABLE_API
#include <gst/webrtc/webrtc.h>

// GIO names a struct member `signals`, which Qt defines as a keyword.
#pragma push_macro("signals")
#undef signals
#include <nice/agent.h>
#pragma pop_macro("signals")

#include <gst/video/video.h>

namespace {

/// Audio only. The microphone in the echo canceller's 10 ms frames, nothing
/// dropped before it; the volume element after it, so a muted call sends silence.
/// No role: the stored recording level is per app name, a role has none.
#define XMATIC_MIC_BRANCH \
    "pulsesrc ! audioconvert ! audioresample ! " \
    "audio/x-raw,format=S16LE,rate=48000,channels=1 ! audiobuffersplit output-buffer-duration=1/100 ! " \
    "identity name=aec ! volume name=micvolume ! queue ! " \
    "opusenc audio-type=voice bitrate-type=cbr bitrate=32000 frame-size=20 inband-fec=true ! " \
    "rtpopuspay pt=111 ! application/x-rtp,media=audio,encoding-name=OPUS,payload=111 ! sendrecv. "

const char *kAudioPipeline =
    "webrtcbin name=sendrecv bundle-policy=max-bundle latency=100 " XMATIC_MIC_BRANCH;

/// The same plus a camera, VP8: the device has no H.264 encoder and Matrix
/// negotiates VP8. A 32-bit build means an old SoC - QVGA at 15 fps.
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

/// Assembled at run time so size, rate and bitrate flow in. The scale caps pin
/// I420, which the encoder needs and the self-view branch then gets free.
QByteArray videoPipelineDescription()
{
    return QStringLiteral(
               "webrtcbin name=sendrecv bundle-policy=max-bundle latency=100 " XMATIC_MIC_BRANCH
               "appsrc name=camsrc is-live=true do-timestamp=true format=time "
               "max-bytes=2000000 ! "
               // max-rate on the element, not a fixed rate in the caps: the camera declares
               // none, and drop-only videorate cannot promise one - the pipeline dies.
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
               // `clock-rate` is what RTP video runs at and belongs in the caps.
               // It does *not* fix the refused video line below - measured - but
               // leaving it out would be wrong on its own.
               "application/x-rtp,media=video,encoding-name=VP8,payload=96,clock-rate=90000 ! "
               "sendrecv. "
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

/// Which decoders a call may plug in - Opus and VP8. Without a list the other
/// side picks any of the phone's decoders for its RTP, gst-droid included.
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

/// Candidates held before an answer, either side. A call gathers a few dozen.
const int kHeldCandidates = 100;
/// Hidden calls remembered at once; the oldest goes first.
const int kHiddenCalls = 16;

/// A random call id from the kernel, not from a clock: everybody in the room
/// sees it, and a guessable id is one somebody else can answer.
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

/// "relay/udp/v4" from an ICE candidate line: what the trace may say about
/// it. The address itself stays out of the journal.
QString candidateKind(const QString &line)
{
    const QStringList parts = line.trimmed().split(QLatin1Char(' '), QString::SkipEmptyParts);
    const int typ = parts.indexOf(QStringLiteral("typ"));
    if (parts.size() < 6 || typ < 0 || typ + 1 >= parts.size()) {
        return QStringLiteral("unreadable");
    }
    const QString &address = parts.at(4);
    const QString family = address.contains(QLatin1Char(':')) ? QStringLiteral("v6")
        : !address.isEmpty() && address.at(0).isDigit() ? QStringLiteral("v4")
        : QStringLiteral("name");
    return parts.at(typ + 1) + QLatin1Char('/') + parts.at(2).toLower() + QLatin1Char('/') + family;
}

QString describeKinds(const QMap<QString, int> &kinds)
{
    if (kinds.isEmpty()) {
        return QStringLiteral("none");
    }
    QStringList out;
    for (auto it = kinds.constBegin(); it != kinds.constEnd(); ++it) {
        out.append(QStringLiteral("%1 x%2").arg(it.key()).arg(it.value()));
    }
    return out.join(QStringLiteral(", "));
}

} // namespace

/// One pipeline's share of the engine, and the user data of every callback it runs.
/// A released pipeline outlives its call: its threads reach this, never the engine.
struct PipelineContext
{
    PipelineContext(CallEngine *engine, GstElement *bin, int generation, bool withVideo)
        : generation(generation), withVideo(withVideo), m_engine(engine), m_bin(bin)
    {
    }
    ~PipelineContext() { release(); }

    const int generation;
    const bool withVideo;
    /// Set before the pipeline runs, read-only after.
    std::shared_ptr<EchoCanceller> echo;
    QAtomicInt remotePackets;
    QAtomicInt cameraFrames;

    /// Takes over the reference.
    void adoptWebrtc(GstElement *webrtc)
    {
        QMutexLocker locker(&m_lock);
        m_webrtc = webrtc;
    }
    /// A new reference, or null once released.
    GstElement *bin() { return referenced(m_bin); }
    GstElement *webrtc() { return referenced(m_webrtc); }

    /// Runs `deliver` with the engine while this pipeline is the current one.
    template <typename Deliver>
    void post(Deliver deliver)
    {
        QMutexLocker locker(&m_lock);
        if (m_engine) {
            deliver(m_engine);
        }
    }
    void detach()
    {
        QMutexLocker locker(&m_lock);
        m_engine = nullptr;
    }
    bool attached()
    {
        QMutexLocker locker(&m_lock);
        return m_engine != nullptr;
    }

    /// Stops the pipeline and drops its references; idempotent, any thread.
    void release()
    {
        GstElement *bin = nullptr;
        GstElement *webrtc = nullptr;
        {
            QMutexLocker locker(&m_lock);
            m_engine = nullptr;
            std::swap(bin, m_bin);
            std::swap(webrtc, m_webrtc);
        }
        // Outside the lock: NULL joins threads that may be waiting for it.
        if (webrtc) {
            gst_object_unref(webrtc);
        }
        if (bin) {
            gst_element_set_state(bin, GST_STATE_NULL);
            gst_object_unref(bin);
        }
    }

    // User data for GLib: a heap copy of the shared pointer, freed with the handler.
    static gpointer share(const std::shared_ptr<PipelineContext> &context)
    {
        return new std::shared_ptr<PipelineContext>(context);
    }
    static const std::shared_ptr<PipelineContext> &shared(gpointer data)
    {
        return *static_cast<std::shared_ptr<PipelineContext> *>(data);
    }
    static PipelineContext *of(gpointer data) { return shared(data).get(); }
    static void unshare(gpointer data) { delete static_cast<std::shared_ptr<PipelineContext> *>(data); }
    static void unshareClosure(gpointer data, GClosure *) { unshare(data); }

private:
    GstElement *referenced(GstElement *const &element)
    {
        QMutexLocker locker(&m_lock);
        return element ? GST_ELEMENT(gst_object_ref(element)) : nullptr;
    }

    QMutex m_lock;
    CallEngine *m_engine;
    GstElement *m_bin;
    GstElement *m_webrtc = nullptr;
};

namespace {

void connectContext(GstElement *element, const char *signal, GCallback callback,
                    const std::shared_ptr<PipelineContext> &context)
{
    g_signal_connect_data(element, signal, callback, PipelineContext::share(context),
                          PipelineContext::unshareClosure, GConnectFlags(0));
}

/// A pipeline kept alive until its TURN allocations are given back; dropped
/// at once they hold the server's relay ports until they time out.
struct RelayRelease
{
    std::shared_ptr<PipelineContext> context;
    GSource *timer = nullptr;
    /// The agent's answer and the timer's teardown; the last one frees.
    std::atomic<int> pending{2};
};

void dropRelease(gpointer user)
{
    auto *release = static_cast<RelayRelease *>(user);
    if (release->pending.fetch_sub(1) == 1) {
        g_source_unref(release->timer);
        delete release;
    }
}

// The agent answers on its own thread, the timer fires on the main one.
void relaysReleased(GObject *, GAsyncResult *, gpointer user)
{
    auto *release = static_cast<RelayRelease *>(user);
    release->context->release();
    g_source_destroy(release->timer);
    dropRelease(release);
}

gboolean relayReleaseTimedOut(gpointer user)
{
    static_cast<RelayRelease *>(user)->context->release();
    return G_SOURCE_REMOVE;
}

/// The agent is reached through properties older GStreamer lacks; there the
/// pipeline simply stops.
void releaseRelays(const std::shared_ptr<PipelineContext> &context)
{
    NiceAgent *agent = nullptr;
    GstElement *webrtc = context->webrtc();
    if (webrtc && g_object_class_find_property(G_OBJECT_GET_CLASS(webrtc), "ice-agent")) {
        GObject *ice = nullptr;
        g_object_get(webrtc, "ice-agent", &ice, nullptr);
        if (ice) {
            if (g_object_class_find_property(G_OBJECT_GET_CLASS(ice), "agent")) {
                g_object_get(ice, "agent", &agent, nullptr);
            }
            g_object_unref(ice);
        }
    }
    if (webrtc) {
        gst_object_unref(webrtc);
    }
    if (!agent) {
        context->release();
        return;
    }

    // Paused: no more sound either way while the agent says goodbye.
    if (GstElement *bin = context->bin()) {
        gst_element_set_state(bin, GST_STATE_PAUSED);
        gst_object_unref(bin);
    }
    auto *release = new RelayRelease;
    release->context = context;
    release->timer = g_timeout_source_new(3000);
    g_source_set_callback(release->timer, relayReleaseTimedOut, release, dropRelease);
    g_source_attach(release->timer, nullptr);
    nice_agent_close_async(agent, relaysReleased, release);
    g_object_unref(agent);
}

} // namespace

CallEngine::CallEngine(QObject *parent)
    : QObject(parent)
{
    m_ringTimeout.setSingleShot(true);
    m_ringTimeout.setInterval(45000);
    connect(&m_ringTimeout, &QTimer::timeout, this, [this]() {
        if (m_state != QLatin1String("ringing")) {
            return;
        }
        // Unanswered is not declined: the caller's own lifetime ends it.
        stopQuietly();
    });

    // Queued on purpose: frames arrive on the camera's thread, and pushing
    // them from there races with tearing the pipeline down.
    connect(&m_camera,
            &CameraSource::frameReady,
            this,
            &CallEngine::pushCameraFrame,
            Qt::QueuedConnection);
    // A camera that never delivers would leave the offer unbuilt and the other
    // side would never see the call. A voice call that rings beats that.
    m_cameraWatchdog.setSingleShot(true);
    m_cameraWatchdog.setInterval(7000);
    connect(&m_cameraWatchdog, &QTimer::timeout, this, [this]() {
        if (!m_withVideo || cameraFrames() > 0 || m_state == QLatin1String("idle")) {
            return;
        }
        qWarning("xmatic: the camera produced nothing, falling back to a voice call");
        setStatus(tr("the camera did not start — continuing without video"));

        // Never through idle: the call page closes on idle and the voice call
        // would run on without a way to hang up.
        if (!m_callId.isEmpty()) {
            emit hangupReady(m_roomId, m_callId, m_partyId);
        }
        tearDown();
        startOutgoing(false);
    });

    m_cameraRetry.setSingleShot(true);
    m_cameraRetry.setInterval(3000);
    connect(&m_cameraRetry, &QTimer::timeout, this, [this]() {
        if (!m_withVideo || m_camera.delivering() || m_state == QLatin1String("idle")) {
            return;
        }
        qWarning("xmatic: the camera gives nothing to plain memory; taking the viewfinder's pictures");
        m_camera.close();
        m_gpuCapture = true;
        emit callChanged();
    });

    m_mediaWatch.setInterval(5000);
    connect(&m_mediaWatch, &QTimer::timeout, this, [this]() {
        if (m_state != QLatin1String("active")) {
            return;
        }
        const int seen = remotePackets();
        m_silentTicks = seen == m_lastRemotePackets ? m_silentTicks + 1 : 0;
        m_lastRemotePackets = seen;
        if (m_silentTicks >= 3) {
            qWarning("xmatic: nothing received for 15 s, ending the call");
            markDisconnected(m_generation.load());
        }
    });

    m_relayRefresh.setSingleShot(true);
    connect(&m_relayRefresh, &QTimer::timeout, this, &CallEngine::relayRefreshNeeded);
    m_iceGrace.setSingleShot(true);
    m_iceGrace.setInterval(12000);
    connect(&m_iceGrace, &QTimer::timeout, this, [this]() {
        if (m_state == QLatin1String("connecting")) {
            markDisconnected(m_generation.load());
        }
    });

    m_relayWait.setSingleShot(true);
    m_relayWait.setInterval(4000);
    connect(&m_relayWait, &QTimer::timeout, this, [this]() {
        qWarning("xmatic: no fresh relay credentials, calling with what there is");
        resumeAfterRelay();
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

void CallEngine::setFailure(const QString &failure)
{
    if (m_failure == failure) {
        return;
    }
    m_failure = failure;
    emit failureChanged();
}

void CallEngine::setState(const QString &state)
{
    if (state != QLatin1String("ringing")) {
        m_ringTimeout.stop();
    }

    if (m_state == state) {
        return;
    }
    const bool wasIdle = m_state == QLatin1String("idle");
    m_state = state;
    if (state == QLatin1String("idle")) {
        m_relayRefresh.stop();
    } else if (wasIdle) {
        armRelayRefresh();
    }
    qInfo("xmatic: call state: %s", qPrintable(state));
    emit callChanged();
}

void CallEngine::setTurnServers(const QVariantList &servers, int ttl)
{
    m_turnServers = servers;
    m_relayKnown = true;
    m_relayExpiresAt = ttl > 0 ? QDateTime::currentMSecsSinceEpoch() + qint64(ttl) * 1000 : 0;
    armRelayRefresh();
    resumeAfterRelay();
}

void CallEngine::armRelayRefresh()
{
    if (m_state == QLatin1String("idle") || m_relayExpiresAt == 0) {
        m_relayRefresh.stop();
        return;
    }
    const qint64 left = m_relayExpiresAt - QDateTime::currentMSecsSinceEpoch();
    m_relayRefresh.start(int(qMin<qint64>(qMax<qint64>(60000, left * 3 / 4), INT_MAX)));
}

void CallEngine::relayUnavailable()
{
    resumeAfterRelay();
    // Asked again while a call rings or runs; idle stops the timer.
    if (m_state != QLatin1String("idle")) {
        m_relayRefresh.start(60000);
    }
}

void CallEngine::forgetRelays()
{
    m_turnServers.clear();
    m_relayKnown = false;
    m_relayExpiresAt = 0;
    m_relayRefresh.stop();
}

int CallEngine::cameraFrames() const
{
    return m_ctx ? m_ctx->cameraFrames.load() : 0;
}

int CallEngine::remotePackets() const
{
    return m_ctx ? m_ctx->remotePackets.load() : 0;
}

bool CallEngine::relayStale() const
{
    if (!m_relayKnown) {
        return true;
    }
    return m_relayExpiresAt != 0
        && QDateTime::currentMSecsSinceEpoch() > m_relayExpiresAt - 60000;
}

bool CallEngine::waitForRelay(int next)
{
    if (!relayStale()) {
        return false;
    }
    m_afterRelay = next;
    m_relayWait.start();
    emit relayRefreshNeeded();
    return true;
}

void CallEngine::resumeAfterRelay()
{
    const int next = m_afterRelay;
    m_afterRelay = 0;
    m_relayWait.stop();
    if (next == 1 && m_state == QLatin1String("calling")) {
        startOutgoing(m_withVideo);
    } else if (next == 2 && m_state == QLatin1String("connecting")) {
        answerOffer();
    }
}

void CallEngine::logCandidates(const char *moment) const
{
    qInfo("xmatic: ICE %s; local: %s; remote: %s",
          moment,
          qPrintable(describeKinds(m_localKinds)),
          qPrintable(describeKinds(m_remoteKinds)));
}

void CallEngine::tearDown()
{
    if (m_busWatch != 0) {
        g_source_remove(m_busWatch);
        m_busWatch = 0;
    }
    m_audioRouter.stop();
    m_candidateTimer.stop();
    m_pendingCandidates.clear();
    m_cameraWatchdog.stop();
    m_cameraRetry.stop();
    m_mediaWatch.stop();
    m_camera.close();
    if (m_gpuCapture) {
        m_gpuCapture = false;
        emit callChanged();
    }
    m_grab.reset();
    m_grabPending = false;
    // References from gst_bin_get_by_name; the bin keeps its own.
    if (m_cameraFeed) {
        gst_object_unref(m_cameraFeed);
        m_cameraFeed = nullptr;
    }
    if (m_cameraFlip) {
        gst_object_unref(m_cameraFlip);
        m_cameraFlip = nullptr;
    }
    m_afterRelay = 0;
    m_relayWait.stop();
    m_iceGrace.stop();
    m_heldCandidates.clear();
    if (m_ctx) {
        m_ctx->detach();
        releaseRelays(m_ctx);
        m_ctx.reset();
    }
    m_pipeline = nullptr;
    m_webrtc = nullptr;
    m_audioSource = nullptr;
    // Detached: its threads queue nothing more, and what is in flight carries the old generation.
    m_generation.ref();
}

bool CallEngine::buildPipeline()
{
    tearDown();
    m_localKinds.clear();
    m_remoteKinds.clear();

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
    if (error) {
        g_error_free(error);
    }

    // The context owns the pipeline's and webrtcbin's references.
    m_ctx = std::make_shared<PipelineContext>(this, m_pipeline, m_generation.load(), m_withVideo);
    m_webrtc = gst_bin_get_by_name(GST_BIN(m_pipeline), "sendrecv");
    m_ctx->adoptWebrtc(m_webrtc);
    m_audioSource = gst_bin_get_by_name(GST_BIN(m_pipeline), "micvolume");
    m_ctx->echo = std::make_shared<EchoCanceller>();
    if (GstElement *tap = gst_bin_get_by_name(GST_BIN(m_pipeline), "aec")) {
        if (GstPad *mic = gst_element_get_static_pad(tap, "src")) {
            m_ctx->echo->processCapture(mic);
            gst_object_unref(mic);
        }
        gst_object_unref(tap);
    }
    if (!m_webrtc) {
        reportFailure(tr("webrtcbin is missing"));
        return false;
    }

    applyTurnServers();

    // Without a bus watch a pipeline failure is completely silent: the call
    // simply never connects and nothing says why.
    GstBus *bus = gst_element_get_bus(m_pipeline);
    // A captureless lambda gives the typed pointer without GStreamer types in
    // the header. The id is kept: the watch stays on the main context for good.
    m_busWatch = gst_bus_add_watch(
        bus,
        [](GstBus *b, GstMessage *m, gpointer u) -> gboolean {
            return CallEngine::busMessage(b, m, u) != 0;
        },
        this);
    gst_object_unref(bus);

    connectContext(m_webrtc, "on-negotiation-needed", G_CALLBACK(negotiationNeeded), m_ctx);
    connectContext(m_webrtc, "on-ice-candidate", G_CALLBACK(iceCandidate), m_ctx);
    connectContext(m_webrtc, "pad-added", G_CALLBACK(streamAdded), m_ctx);
    connectContext(m_webrtc, "notify::connection-state", G_CALLBACK(connectionStateChanged), m_ctx);
    g_signal_connect(m_webrtc, "notify::ice-connection-state", G_CALLBACK(iceStateChanged), nullptr);
    g_signal_connect(m_webrtc, "notify::ice-gathering-state", G_CALLBACK(iceStateChanged), nullptr);

    // The bin owns the element; the raw pointer lives as long as the context.
    if (m_audioSource) {
        gst_object_unref(m_audioSource);
    }

    // The context counts what the camera delivers: a video call that never connects
    // looks the same whether the sensor or the negotiation failed.
    m_cameraFormat.clear();
    m_cameraFeed = m_withVideo ? gst_bin_get_by_name(GST_BIN(m_pipeline), "camsrc") : nullptr;
    m_cameraFlip = m_withVideo ? gst_bin_get_by_name(GST_BIN(m_pipeline), "camflip") : nullptr;
    if (m_cameraFlip) {
        setOrientation(m_orientation);
    }
    if (m_withVideo) {
        GstElement *selfSink = gst_bin_get_by_name(GST_BIN(m_pipeline), "selfsink");
        if (selfSink) {
            connectContext(selfSink, "new-sample", G_CALLBACK(selfFrameArrived), m_ctx);
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
                    PipelineContext::share(m_ctx),
                    PipelineContext::unshare);
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
    // One URI per transport: libnice allocates per URI and local address, and
    // a server with the default 21 relay ports runs dry within one call.
    // UDP prefers plain `turn:`, TCP prefers `turns:`.
    QVariant udp;
    QVariant tcp;
    bool udpPlain = false;
    bool tcpTls = false;
    for (const QVariant &entry : m_turnServers) {
        const QString uri = entry.toMap().value(QStringLiteral("uri")).toString();
        const bool tls = uri.startsWith(QLatin1String("turns:"), Qt::CaseInsensitive);
        if (uri.contains(QLatin1String("transport=tcp"), Qt::CaseInsensitive)) {
            if (!tcp.isValid() || (tls && !tcpTls)) {
                tcp = entry;
                tcpTls = tls;
            }
        } else if (!udp.isValid() || (!tls && !udpPlain)) {
            udp = entry;
            udpPlain = !tls;
        }
    }

    for (const QVariant &entry : {udp, tcp}) {
        if (!entry.isValid()) {
            continue;
        }
        const QVariantMap server = entry.toMap();
        const QString uri = server.value(QStringLiteral("uri")).toString();
        const QString user = server.value(QStringLiteral("username")).toString();
        const QString password = server.value(QStringLiteral("password")).toString();
        if (uri.isEmpty()) {
            continue;
        }

        // webrtcbin wants credentials inside the URI, percent-encoded, with the
        // scheme kept: rewriting `turns:` sent the allocation out in the clear.
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
        // The offer waited for exactly this: now webrtcbin can describe the
        // video line instead of refusing it.
        if (m_offerDeferred) {
            requestOffer();
        }
    }

    GstBuffer *buffer = gst_buffer_new_allocate(nullptr, data.size(), nullptr);
    gst_buffer_fill(buffer, 0, data.constData(), data.size());

    GstFlowReturn flow = GST_FLOW_OK;
    g_signal_emit_by_name(m_cameraFeed, "push-buffer", buffer, &flow);
    gst_buffer_unref(buffer);

    if (flow != GST_FLOW_OK && cameraFrames() < 2) {
        qWarning("xmatic: camera frame rejected by the pipeline (%d)", int(flow));
    }
}

void CallEngine::placeCall(const QString &roomId, bool withVideo)
{
    if (!m_available || roomId.isEmpty() || m_state != QLatin1String("idle")) {
        return;
    }

    // A new call starts without the last one's reason: it is what holds the page
    // open, so a stale one would keep the next call's page from closing.
    setFailure(QString());

    // Without a camera the video branch produces no caps, the pipeline does not
    // start and no invitation goes out. Falling back to voice beats silence.
    m_withVideo = withVideo && CameraSource::isAvailable();
    if (withVideo && !m_withVideo) {
        setStatus(tr("no camera — placing a voice call"));
    }

    m_roomId = roomId;
    m_peer.clear();
    setState(QStringLiteral("calling"));
    if (!waitForRelay(1)) {
        startOutgoing(m_withVideo);
    }
}

void CallEngine::startOutgoing(bool withVideo)
{
    m_withVideo = withVideo;
    setSpeaker(withVideo);
    m_offerWaits = 0;
    m_offerDeferred = false;
    m_callId = freshId(QStringLiteral("c"));
    m_partyId = freshId(QStringLiteral("p"));
    m_isCaller = true;
    m_selectedParty.clear();
    m_pendingRemoteOffer.clear();
    emit callChanged();

    if (!buildPipeline()) {
        return;
    }
    if (m_withVideo) {
        m_camera.open();
        m_cameraRetry.start();
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
                                const QString &sdp,
                                bool show,
                                bool audible,
                                qint64 lifetimeMs)
{
    if (!m_available) {
        return;
    }
    // "Calls: off" takes no state: no busy reply to others, nothing sent at all.
    if (!show && callId != m_callId) {
        pruneHiddenCalls();
        if (m_hiddenCalls.size() >= kHiddenCalls) {
            m_hiddenCalls.removeFirst();
        }
        const qint64 lifetime = qBound<qint64>(10000, lifetimeMs, 120000);
        m_hiddenCalls.append({roomId, callId, QDateTime::currentMSecsSinceEpoch() + lifetime});
        qInfo("xmatic: incoming call hidden by the push rules");
        return;
    }
    if (m_state != QLatin1String("idle")) {
        // Already busy: the specification wants a hangup rather than silence.
        emit hangupReady(roomId, callId, freshId(QStringLiteral("p")));
        return;
    }
    setFailure(QString());

    m_roomId = roomId;
    m_peer = sender;
    m_callId = callId;
    m_partyId = freshId(QStringLiteral("p"));
    m_isCaller = false;
    m_selectedParty.clear();
    m_inviteVersion.clear();
    // What was offered, not what will be answered: the camera opens when the user
    // picks the action. The core has already applied the privacy setting.
    m_videoOffered = videoAllowed && CameraSource::isAvailable();
    m_videoRefused = videoOffered && !m_videoOffered;
    // Which of the three decided it: the caller's offer, this device's privacy
    // setting, or the camera. Without this the page just quietly says "Accept",
    // and the pipeline then meets video nobody asked for.
    qInfo("xmatic: incoming call offers video %d, allowed here %d, camera %d",
          videoOffered ? 1 : 0,
          videoAllowed ? 1 : 0,
          CameraSource::isAvailable() ? 1 : 0);
    m_withVideo = false;
    m_pendingRemoteOffer = sdp;
    m_earlyCandidates.clear();
    qInfo("xmatic: incoming call, sound %d", audible ? 1 : 0);
    emit callChanged();

    setState(QStringLiteral("ringing"));
    // Fetched while it rings, so the answer rarely has to wait.
    if (relayStale()) {
        emit relayRefreshNeeded();
    }
    // A ring that nobody answers ends by itself. Without this the ringtone
    // loops until the caller gives up - and a caller can choose not to.
    m_ringTimeout.start();
    emit incomingCall(roomId, sender, audible);
}

void CallEngine::pruneHiddenCalls()
{
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    for (int index = m_hiddenCalls.size() - 1; index >= 0; --index) {
        if (m_hiddenCalls.at(index).until <= now) {
            m_hiddenCalls.removeAt(index);
        }
    }
}

bool CallEngine::isHiddenCall(const QString &roomId, const QString &callId)
{
    pruneHiddenCalls();
    for (const HiddenCall &call : m_hiddenCalls) {
        if (call.callId == callId && call.roomId == roomId) {
            return true;
        }
    }
    return false;
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

    setSpeaker(m_withVideo);
    setState(QStringLiteral("connecting"));
    if (!waitForRelay(2)) {
        answerOffer();
    }
}

void CallEngine::answerOffer()
{
    if (!buildPipeline()) {
        return;
    }
    if (m_withVideo) {
        m_camera.open();
        m_cameraRetry.start();
    }

    if (gst_element_set_state(m_pipeline, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE) {
        reportFailure(tr("the microphone could not be opened"));
        return;
    }

    // Answering means taking their offer first; the answer follows from it.
    applyRemoteDescription(m_pendingRemoteOffer, true);
    m_pendingRemoteOffer.clear();

    const QVariantList early = m_earlyCandidates;
    m_earlyCandidates.clear();
    if (!early.isEmpty()) {
        qInfo("xmatic: %d candidates held while it rang", early.size());
    }
    for (const QVariant &entry : early) {
        addRemoteCandidate(entry.toMap());
    }
}

void CallEngine::onAnsweredElsewhere(const QString &roomId, const QString &callId,
                                     const QString &partyId, bool declined)
{
    if (isHiddenCall(roomId, callId)) {
        return;
    }
    // Only while this device is still offering to answer, and only for the call
    // it is ringing about. Our own answer carries our own party id and must not
    // stop us.
    if (m_state != QLatin1String("ringing") || callId != m_callId || roomId != m_roomId) {
        return;
    }
    if (!partyId.isEmpty() && partyId == m_partyId) {
        return;
    }

    qInfo("xmatic: the call was %s on another device", declined ? "declined" : "answered");
    setStatus(declined ? tr("declined on another device") : tr("answered on another device"));
    stopQuietly();
}

void CallEngine::onSelectAnswer(const QString &roomId, const QString &sender,
                                const QString &callId, const QString &selectedPartyId)
{
    if (isHiddenCall(roomId, callId)) {
        return;
    }
    // Callee side only, and only the caller may choose: the call id is public.
    if (m_isCaller || callId != m_callId || roomId != m_roomId || m_state == QLatin1String("idle")
        || sender.isEmpty() || sender != m_peer
        || selectedPartyId.isEmpty() || selectedPartyId == m_partyId) {
        return;
    }
    qInfo("xmatic: the caller took the answer of another device");
    setStatus(tr("answered on another device"));
    stopQuietly();
}

void CallEngine::noteInviteVersion(const QString &callId, const QString &version)
{
    if (callId == m_callId) {
        m_inviteVersion = version;
    }
}

/// Ends the call here without a word on the wire: another party already spoke.
void CallEngine::stopQuietly()
{
    tearDown();
    m_remoteVideo.stop();
    m_withVideo = false;
    m_callId.clear();
    m_roomId.clear();
    m_peer.clear();
    m_pendingRemoteOffer.clear();
    m_earlyCandidates.clear();
    setState(QStringLiteral("idle"));
    emit callChanged();
}

void CallEngine::onRemoteAnswer(const QString &roomId,
                               const QString &sender,
                               const QString &callId,
                               const QString &partyId,
                               const QString &sdp)
{
    // The id alone is not authentication - everybody in the room sees it. The
    // first answer decides the party, and every later event must match it.
    if (callId != m_callId || !m_isCaller || roomId != m_roomId) {
        return;
    }
    // Unset peer: a group room, or the reply not in yet; the core holds a room
    // of two to its other member, and this account's answers never come here.
    if (!m_peer.isEmpty() && sender != m_peer) {
        return;
    }
    // A second answering device of the same person: the first one was taken.
    if (!m_selectedParty.isEmpty()) {
        return;
    }
    m_peer = sender;
    m_selectedParty = partyId.isEmpty() ? QStringLiteral("-") : partyId;
    if (!partyId.isEmpty()) {
        emit selectAnswerReady(m_roomId, m_callId, m_partyId, partyId);
    }
    setState(QStringLiteral("connecting"));
    applyRemoteDescription(sdp, false);

    if (!m_heldCandidates.isEmpty()) {
        qInfo("xmatic: %d own candidates sent after the answer", m_heldCandidates.size());
        m_pendingCandidates.append(m_heldCandidates);
        m_heldCandidates.clear();
        m_candidateTimer.stop();
        flushCandidates();
    }
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
    for (const QString &line : sdp.split(QLatin1Char('\n'))) {
        if (line.startsWith(QLatin1String("a=candidate:"))) {
            ++m_remoteKinds[candidateKind(line.mid(2))];
        }
    }

    // Which lines carry video, read before the description takes the message
    // over: answering an offer we have no video pipeline for needs them below,
    // and after `gst_webrtc_session_description_free` the message is gone.
    QVector<guint> videoLines;
    if (isOffer && !m_withVideo) {
        const guint medias = gst_sdp_message_medias_len(message);
        for (guint index = 0; index < medias; ++index) {
            const GstSDPMedia *media = gst_sdp_message_get_media(message, index);
            const gchar *kind = media ? gst_sdp_media_get_media(media) : nullptr;
            if (kind && g_strcmp0(kind, "video") == 0) {
                videoLines.append(index);
            }
        }
    }

    GstWebRTCSessionDescription *description = gst_webrtc_session_description_new(
        isOffer ? GST_WEBRTC_SDP_TYPE_OFFER : GST_WEBRTC_SDP_TYPE_ANSWER,
        message);

    GstPromise *promise = gst_promise_new();
    g_signal_emit_by_name(m_webrtc, "set-remote-description", description, promise);
    gst_promise_interrupt(promise);
    gst_promise_unref(promise);
    gst_webrtc_session_description_free(description);

    // Refuse their video line instead of leaving it dangling. Accepting a video
    // call "without camera" builds the audio-only pipeline, so nothing takes
    // from the transport webrtcbin starts for that line: measured on the device,
    // `nicesrc: Internal data stream error` a second after the call went active,
    // and `bundle-policy=max-bundle` takes the audio down with it. The whole
    // call ended, which read as the other side refusing the answer.
    for (guint index : videoLines) {
        GstWebRTCRTPTransceiver *transceiver = nullptr;
        g_signal_emit_by_name(m_webrtc, "get-transceiver", index, &transceiver);
        if (!transceiver) {
            continue;
        }
        g_object_set(transceiver,
                     "direction",
                     GST_WEBRTC_RTP_TRANSCEIVER_DIRECTION_INACTIVE,
                     nullptr);
        gst_object_unref(transceiver);
    }

    if (isOffer) {
        // Now that their offer is known, ask for our answer.
        GstPromise *answer = gst_promise_new_with_change_func(
            descriptionCreated, PipelineContext::share(m_ctx), PipelineContext::unshare);
        g_signal_emit_by_name(m_webrtc, "create-answer", nullptr, answer);
    }
}

void CallEngine::onRemoteCandidates(const QString &roomId,
                                   const QString &sender,
                                   const QString &callId,
                                   const QVariantList &candidates)
{
    if (isHiddenCall(roomId, callId)) {
        return;
    }
    // Same rule as the answer: an injected candidate makes this device probe an
    // address of somebody else's choosing. Before the party is known, the room is the check.
    if (callId != m_callId || roomId != m_roomId) {
        return;
    }
    if (!m_peer.isEmpty() && sender != m_peer) {
        return;
    }
    // The caller sends them right after the invite; dropped, only a peer that
    // can reach this one directly ever connects - the same Wi-Fi, never two carriers.
    if (!m_webrtc) {
        const int room = kHeldCandidates - m_earlyCandidates.size();
        if (!m_isCaller && m_state != QLatin1String("idle") && room > 0) {
            m_earlyCandidates.append(candidates.mid(0, room));
        }
        return;
    }

    for (const QVariant &entry : candidates) {
        addRemoteCandidate(entry.toMap());
    }
}

void CallEngine::addRemoteCandidate(const QVariantMap &candidate)
{
    const QString value = candidate.value(QStringLiteral("candidate")).toString();
    if (value.isEmpty() || !m_webrtc) {
        // The end-of-candidates marker; nothing to add.
        return;
    }
    const unsigned index = candidate.value(QStringLiteral("sdpMLineIndex")).toUInt();
    ++m_remoteKinds[candidateKind(value)];
    g_signal_emit_by_name(m_webrtc, "add-ice-candidate", index, value.toUtf8().constData());
}

void CallEngine::onRemoteHangup(const QString &roomId,
                               const QString &sender,
                               const QString &callId,
                               const QString &partyId,
                               bool reject)
{
    if (isHiddenCall(roomId, callId)) {
        return;
    }
    // Once an answer is taken, a reject is void and only that device may hang up.
    if (m_isCaller && !m_selectedParty.isEmpty() && callId == m_callId) {
        if (reject) {
            return;
        }
        // A device without a party id (version 0) is not the chosen one either.
        if (m_selectedParty != QLatin1String("-") && partyId != m_selectedParty) {
            return;
        }
    }
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
    // Only a hangup naming this exact call ends it. The loose match let stale
    // hangups replay on sync and kill a freshly placed call within a second.
    if (callId.isEmpty() || m_callId.isEmpty() || callId != m_callId) {
        qInfo("xmatic: ignoring hangup for another call");
        return;
    }
    if (m_state == QLatin1String("connecting")) {
        logCandidates("ended by the other side");
    }
    setStatus(tr("the other side hung up"));
    tearDown();
    m_remoteVideo.stop();
    m_withVideo = false;
    m_callId.clear();
    m_roomId.clear();
    m_peer.clear();
    m_pendingRemoteOffer.clear();
    m_earlyCandidates.clear();
    setState(QStringLiteral("idle"));
    emit callChanged();
}

void CallEngine::hangUp()
{
    if (m_state == QLatin1String("idle")) {
        return;
    }
    if (m_state == QLatin1String("connecting")) {
        logCandidates("given up");
    }

    if (!m_roomId.isEmpty() && !m_callId.isEmpty()) {
        // Declining one that never rang through is a reject: it also stops the
        // account's other devices ringing.
        if (!m_isCaller && m_state == QLatin1String("ringing")
            && m_inviteVersion != QLatin1String("0")) {
            emit rejectReady(m_roomId, m_callId, m_partyId);
        } else {
            emit hangupReady(m_roomId, m_callId, m_partyId);
        }
    }

    tearDown();
    m_callId.clear();
    m_roomId.clear();
    m_peer.clear();
    m_pendingRemoteOffer.clear();
    m_earlyCandidates.clear();
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

    // GstVideoFlipMethod: 0 none, 1 quarter clockwise, 2 half, 3 quarter
    // anticlockwise. The sensor sits a quarter turn from the upright phone.
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

    m_flipMethod = method;
    g_object_set(m_cameraFlip, "method", method, nullptr);
    qInfo("xmatic: camera rotation method %d for orientation %d", method, orientation);
}

bool CallEngine::grabFrame(QQuickItem *item)
{
    if (!item || m_grabPending) {
        return false;
    }
    // Replacing the last result frees it; its ready signal is long past.
    m_grab = item->grabToImage(QSize(480, 640));
    if (!m_grab) {
        return false;
    }
    m_grabPending = true;
    QQuickItemGrabResult *grab = m_grab.data();
    connect(grab, &QQuickItemGrabResult::ready, this, [this, grab]() {
        m_grabPending = false;
        pushGrabbedFrame(grab->image());
    });
    return true;
}

void CallEngine::pushGrabbedFrame(const QImage &image)
{
    if (!m_gpuCapture || image.isNull()) {
        return;
    }
    // The grab is the front viewfinder: fixed to the device like the sensor, but
    // mirrored. Unmirrored and laid as the sensor lies, the flip does the rest.
    QTransform turn;
    turn.rotate(90);
    const QImage frame = image.mirrored(true, false).transformed(turn).convertToFormat(QImage::Format_RGB32);
    const QByteArray data(reinterpret_cast<const char *>(frame.constBits()),
                          frame.bytesPerLine() * frame.height());
    pushCameraFrame(data, frame.width(), frame.height(), QStringLiteral("BGRx"));
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

void CallEngine::setSpeaker(bool on)
{
    m_audioRouter.setSpeaker(on);
    if (m_speaker != on) {
        m_speaker = on;
        emit speakerChanged();
    }
}

void CallEngine::negotiationNeeded(GstElement *, void *user)
{
    const int generation = PipelineContext::of(user)->generation;
    PipelineContext::of(user)->post([generation](CallEngine *engine) {
        QMetaObject::invokeMethod(engine, "handleNegotiationNeeded", Qt::QueuedConnection,
                                  Q_ARG(int, generation));
    });
}

void CallEngine::handleNegotiationNeeded(int generation)
{
    if (!current(generation)) {
        return;
    }
    if (!m_isCaller) {
        // The answering side negotiates from the remote offer, not on its own.
        return;
    }

    // The camera's first frame is what gives the appsrc its caps (see
    // `pushCameraFrame`), so wait for it before describing the video line.
    // A precaution, not the cure for `m=video 0` - that was our own reader
    // (`offers_active_video`), and the offer's shape is the same either way.
    if (m_withVideo && m_cameraFormat.isEmpty()) {
        m_offerDeferred = true;
        // A camera that never delivers must not hold the call: after this the
        // offer goes as it is, which is a voice call - degrade, not block.
        QTimer::singleShot(7000, this, [this, generation]() {
            if (current(generation) && m_offerDeferred) {
                qWarning("xmatic: camera sent nothing in time, offering voice only");
                requestOffer();
            }
        });
        return;
    }
    requestOffer();
}

bool CallEngine::sendPadsReady() const
{
    if (!m_webrtc) {
        return true;
    }
    bool ready = true;
    GstIterator *pads = gst_element_iterate_sink_pads(m_webrtc);
    GValue item = G_VALUE_INIT;
    bool done = false;
    while (!done) {
        switch (gst_iterator_next(pads, &item)) {
        case GST_ITERATOR_OK: {
            auto *pad = GST_PAD(g_value_get_object(&item));
            GstCaps *caps = pad ? gst_pad_get_current_caps(pad) : nullptr;
            if (!caps) {
                ready = false;
                done = true;
            } else {
                gst_caps_unref(caps);
            }
            g_value_reset(&item);
            break;
        }
        case GST_ITERATOR_RESYNC:
            ready = true;
            gst_iterator_resync(pads);
            break;
        default:
            done = true;
            break;
        }
    }
    g_value_unset(&item);
    gst_iterator_free(pads);
    return ready;
}

void CallEngine::requestOffer()
{
    // Not after the call is over: the deferred path can fire from a timer, and
    // by then the user may have hung up.
    if (!m_webrtc || !m_isCaller || m_state == QLatin1String("idle")) {
        m_offerDeferred = false;
        return;
    }

    // The first frame is not enough: it still has to travel through the
    // converter, the encoder and the payloader before webrtcbin's own pad knows
    // what it carries. Same precaution as above, and the same caveat: it never
    // was what made a video line read as refused.
    if (m_withVideo && m_offerWaits < 30 && !sendPadsReady()) {
        ++m_offerWaits;
        m_offerDeferred = true;
        const int generation = m_generation.load();
        QTimer::singleShot(100, this, [this, generation]() {
            if (current(generation) && m_offerDeferred) {
                requestOffer();
            }
        });
        return;
    }
    if (m_withVideo && !sendPadsReady()) {
        qWarning("xmatic: the video branch never described itself, offering voice only");
    }
    m_offerDeferred = false;

    GstPromise *promise = gst_promise_new_with_change_func(
        descriptionCreated, PipelineContext::share(m_ctx), PipelineContext::unshare);
    g_signal_emit_by_name(m_webrtc, "create-offer", nullptr, promise);
}

void CallEngine::descriptionCreated(GstPromise *promise, void *user)
{
    // A copy: unreferencing the promise can free `user`.
    const std::shared_ptr<PipelineContext> context = PipelineContext::shared(user);
    const int generation = context->generation;

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
        context->post([generation](CallEngine *engine) {
            QMetaObject::invokeMethod(engine, "reportFailureFor", Qt::QueuedConnection,
                                      Q_ARG(int, generation),
                                      Q_ARG(QString, QObject::tr("no session description")));
        });
        return;
    }

    // Ours has to be applied locally before it is sent, or the candidates
    // gathered afterwards belong to nothing. Its own webrtcbin, never the next call's.
    GstElement *webrtc = context->attached() ? context->webrtc() : nullptr;
    if (!webrtc) {
        gst_webrtc_session_description_free(description);
        return;
    }
    GstPromise *applied = gst_promise_new();
    g_signal_emit_by_name(webrtc, "set-local-description", description, applied);
    gst_promise_interrupt(applied);
    gst_promise_unref(applied);
    gst_object_unref(webrtc);

    gchar *text = gst_sdp_message_as_text(description->sdp);
    const QString sdp = QString::fromUtf8(text);
    g_free(text);
    gst_webrtc_session_description_free(description);

    // What we are about to send, media lines and their attributes only: no `c=`,
    // no candidates, no fingerprints - those carry addresses and keys. This is
    // the one place that can say why a video line comes out refused, and four
    // measurements have already ruled out pipeline, transceiver, pad caps and
    // timing.
    {
        QStringList shape;
        const QStringList lines = sdp.split(QLatin1Char('\n'));
        for (const QString &raw : lines) {
            const QString line = raw.trimmed();
            if (line.startsWith(QLatin1String("m="))) {
                shape.append(line);
                continue;
            }
            if (!line.startsWith(QLatin1String("a="))) {
                continue;
            }
            if (line.startsWith(QLatin1String("a=candidate"))
                || line.startsWith(QLatin1String("a=fingerprint"))
                || line.startsWith(QLatin1String("a=ice-"))
                || line.startsWith(QLatin1String("a=msid"))
                || line.startsWith(QLatin1String("a=ssrc"))) {
                continue;
            }
            shape.append(line);
        }
        qInfo("xmatic: local %s shape: %s",
              isOffer ? "offer" : "answer",
              qPrintable(shape.join(QStringLiteral(" | "))));
    }

    context->post([generation, sdp, isOffer](CallEngine *engine) {
        QMetaObject::invokeMethod(engine, "deliverLocalDescription", Qt::QueuedConnection,
                                  Q_ARG(int, generation), Q_ARG(QString, sdp),
                                  Q_ARG(bool, isOffer));
    });
}

void CallEngine::iceCandidate(GstElement *, unsigned index, char *candidate, void *user)
{
    PipelineContext *context = PipelineContext::of(user);
    const int generation = context->generation;
    const QString line = QString::fromUtf8(candidate);
    context->post([generation, line, index](CallEngine *engine) {
        QMetaObject::invokeMethod(engine, "deliverCandidate", Qt::QueuedConnection,
                                  Q_ARG(int, generation), Q_ARG(QString, line),
                                  Q_ARG(int, static_cast<int>(index)));
    });
}

void CallEngine::streamAdded(GstElement *, GstPad *pad, void *user)
{
    const std::shared_ptr<PipelineContext> &context = PipelineContext::shared(user);
    if (GST_PAD_DIRECTION(pad) != GST_PAD_SRC || !context->attached()) {
        return;
    }
    // Its own bin: a released pipeline's stream must not land in the next call's.
    std::unique_ptr<GstElement, void (*)(gpointer)> hold(context->bin(), gst_object_unref);
    GstElement *bin = hold.get();
    if (!bin) {
        return;
    }

    // The stream arrives as RTP; decodebin announces a pad once it knows whether
    // this is audio or video, which is where the branches part.
    GstElement *decode = gst_element_factory_make("decodebin", nullptr);
    if (!decode) {
        return;
    }
    // Skip the Android hardware decoder for incoming VP8: droidvdec refuses the
    // caps and takes the whole receive transport, audio included, down with it.
    g_object_set(decode, "force-sw-decoders", TRUE, nullptr);
    g_signal_connect(decode, "autoplug-select", G_CALLBACK(chooseDecoder), nullptr);

    gst_bin_add(GST_BIN(bin), decode);
    // Connect the handler and link the incoming pad BEFORE decodebin starts,
    // then sync it to PLAYING last — same not-linked race as the branches.
    connectContext(decode, "pad-added", G_CALLBACK(decodedPadAdded), context);

    gst_pad_add_probe(
        pad,
        GST_PAD_PROBE_TYPE_BUFFER,
        [](GstPad *p, GstPadProbeInfo *i, gpointer u) -> GstPadProbeReturn {
            return static_cast<GstPadProbeReturn>(CallEngine::remoteProbe(p, i, u));
        },
        PipelineContext::share(context),
        PipelineContext::unshare);

    GstPad *decodeSink = gst_element_get_static_pad(decode, "sink");
    if (decodeSink) {
        gst_pad_link(pad, decodeSink);
        gst_object_unref(decodeSink);
    }

    gst_element_sync_state_with_parent(decode);
}

void CallEngine::decodedPadAdded(GstElement *, GstPad *pad, void *user)
{
    const std::shared_ptr<PipelineContext> &context = PipelineContext::shared(user);
    if (!context->attached()) {
        return;
    }
    std::unique_ptr<GstElement, void (*)(gpointer)> hold(context->bin(), gst_object_unref);
    GstElement *bin = hold.get();
    if (!bin) {
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

    // Video we did not ask for: the SDP answer refuses the line, but a pad can
    // still arrive - a renegotiation, a client that ignores the refusal, or the
    // refusal landing too late. Returning here leaves the pad unlinked, and the
    // comment below says what that costs: `internal data stream error`, which
    // takes the whole call down, audio included. Measured on the device, three
    // times, always a second after the call went active. So it is drained
    // instead of dropped: a fakesink is a destination, a missing link is not.
    if (isVideo && !context->withVideo) {
        GstElement *drain = gst_element_factory_make("fakesink", nullptr);
        if (!drain) {
            return;
        }
        g_object_set(drain, "sync", FALSE, "async", FALSE, nullptr);
        gst_bin_add(GST_BIN(bin), drain);
        GstPad *sink = gst_element_get_static_pad(drain, "sink");
        if (!sink || gst_pad_link(pad, sink) != GST_PAD_LINK_OK) {
            if (sink) {
                gst_object_unref(sink);
            }
            gst_bin_remove(GST_BIN(bin), drain);
            return;
        }
        gst_object_unref(sink);
        gst_element_sync_state_with_parent(drain);
        qInfo("xmatic: incoming video not asked for, drained");
        return;
    }

    GstElement *first = nullptr;
    GstElement *last = nullptr;
    // The branch elements, started only after everything is linked: pushing into
    // an unlinked pad surfaces as "internal data stream error" and kills the call.
    GstElement *chain[7] = {};
    int chainN = 0;

    if (isVideo) {
        // Straight to RGB - the frames go to Qt as images. A leaky queue keeps a slow
        // video decoder from stalling the shared transport and the audio with it.
        GstElement *queue = gst_element_factory_make("queue", nullptr);
        GstElement *convert = gst_element_factory_make("videoconvert", nullptr);
        GstElement *sink = gst_element_factory_make("appsink", nullptr);
        if (!queue || !convert || !sink) {
            // What was created has to be released by hand: the elements only reach the bin
            // further down. Missing plugins are the only way in, so the leak is bounded.
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

        // I420 is what the decoder produces, so the convert in front is a passthrough.
        // No size here: downstream caps cannot shrink an allocation, only fail it.
        GstCaps *wanted = gst_caps_new_simple("video/x-raw",
                                              "format", G_TYPE_STRING, "I420",
                                              nullptr);
        g_object_set(sink, "caps", wanted, "emit-signals", TRUE, "max-buffers", 2,
                     "drop", TRUE, nullptr);
        gst_caps_unref(wanted);

        connectContext(sink, "new-sample", G_CALLBACK(videoFrameArrived), context);

        gst_bin_add_many(GST_BIN(bin), queue, convert, sink, nullptr);
        gst_element_link_many(queue, convert, sink, nullptr);

        first = queue;
        last = sink;
        chain[chainN++] = queue;
        chain[chainN++] = convert;
        chain[chainN++] = sink;
    } else {
        // The queue gives the audio branch its own thread, so a busy video path can no
        // longer starve it - silent audio in video calls, voice-only fine.
        GstElement *queue = gst_element_factory_make("queue", nullptr);
        GstElement *convert = gst_element_factory_make("audioconvert", nullptr);
        GstElement *resample = gst_element_factory_make("audioresample", nullptr);
        // The echo canceller's format and frames; it reads what enters the sink.
        GstElement *format = gst_element_factory_make("capsfilter", nullptr);
        if (format) {
            GstCaps *caps = gst_caps_from_string("audio/x-raw,format=S16LE,rate=48000,channels=1");
            g_object_set(format, "caps", caps, nullptr);
            gst_caps_unref(caps);
        }
        GstElement *split = gst_element_factory_make("audiobuffersplit", nullptr);
        if (split) {
            gst_util_set_object_arg(G_OBJECT(split), "output-buffer-duration", "1/100");
        }
        GstElement *tap = gst_element_factory_make("identity", nullptr);
        // pulsesink, not autoaudiosink: the default sink on these devices is
        // sink.null, so received audio played into nothing.
        GstElement *sink = gst_element_factory_make("pulsesink", nullptr);
        // The name lets the router find the stream; the role is the call's and
        // bypasses the muted "x-maemo" entry of stream-restore.
        if (sink) {
            GstStructure *props = gst_structure_new("props",
                                                    "application.name", G_TYPE_STRING, "xmatic call",
                                                    "media.role", G_TYPE_STRING, "phone",
                                                    nullptr);
            g_object_set(sink, "stream-properties", props, nullptr);
            gst_structure_free(props);
        }
        if (!queue || !convert || !resample || !format || !split || !tap || !sink) {
            return;
        }
        gst_bin_add_many(GST_BIN(bin), queue, convert, resample, format, split, tap,
                         sink, nullptr);
        gst_element_link_many(queue, convert, resample, format, split, tap, sink, nullptr);
        if (context->echo) {
            if (GstPad *played = gst_element_get_static_pad(tap, "src")) {
                context->echo->watchPlayback(played);
                gst_object_unref(played);
            }
        }

        first = queue;
        last = sink;
        chain[chainN++] = queue;
        chain[chainN++] = convert;
        chain[chainN++] = resample;
        chain[chainN++] = format;
        chain[chainN++] = split;
        chain[chainN++] = tap;
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
    PipelineContext *context = PipelineContext::of(user);
    const int generation = context->generation;

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
            // What the other side sends decides this allocation, so it is bounded here.
            // A call that sends 4K to a phone is not a call.
            if (width <= 0 || height <= 0 || width > kMaximumRemoteWidth
                || height > kMaximumRemoteHeight) {
                gst_video_frame_unmap(&frame);
                gst_sample_unref(sample);
                return GST_FLOW_OK;
            }
            // Repack into tight I420: the decoder's planes may be padded to an alignment,
            // and a QVideoFrame wants contiguous rows.
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
            context->post([generation, &tight, width, height](CallEngine *engine) {
                QMetaObject::invokeMethod(engine, "deliverVideoYuv", Qt::QueuedConnection,
                                          Q_ARG(int, generation), Q_ARG(QByteArray, tight),
                                          Q_ARG(int, width), Q_ARG(int, height));
            });
        }
    }

    gst_sample_unref(sample);
    return 0;
}

void CallEngine::deliverVideoFrame(const QImage &frame)
{
    // Frames are queued from the GStreamer thread, so one that was already in
    // flight lands after `stop()` and starts the surface again - `active` stays
    // true and the next call's page draws its video layout while ringing.
    if (m_state == QLatin1String("idle")) {
        return;
    }
    m_remoteVideo.present(frame);
}

void CallEngine::deliverVideoYuv(int generation, const QByteArray &planes, int width,
                                 int height)
{
    if (!current(generation) || m_state == QLatin1String("idle")) {
        return;
    }
    m_remoteVideo.presentYuv(planes, QSize(width, height));
}

void CallEngine::deliverSelfYuv(int generation, const QByteArray &planes, int width,
                                int height)
{
    if (!current(generation)) {
        return;
    }
    m_selfVideo.presentYuv(planes, QSize(width, height));
}

int CallEngine::selfFrameArrived(void *sink, void *user)
{
    PipelineContext *context = PipelineContext::of(user);
    const int generation = context->generation;

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
            context->post([generation, &tight, width, height](CallEngine *engine) {
                QMetaObject::invokeMethod(engine, "deliverSelfYuv", Qt::QueuedConnection,
                                          Q_ARG(int, generation), Q_ARG(QByteArray, tight),
                                          Q_ARG(int, width), Q_ARG(int, height));
            });
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

    // A failed or closed connection has to return the engine to idle, or the call
    // never ends here and every later one is refused as busy.
    const char *slot = state == GST_WEBRTC_PEER_CONNECTION_STATE_CONNECTED ? "markConnected"
        : state == GST_WEBRTC_PEER_CONNECTION_STATE_FAILED                 ? "markFailed"
        : state == GST_WEBRTC_PEER_CONNECTION_STATE_CLOSED                 ? "markDisconnected"
                                                                           : nullptr;
    if (!slot) {
        return;
    }
    PipelineContext *context = PipelineContext::of(user);
    const int generation = context->generation;
    context->post([slot, generation](CallEngine *engine) {
        QMetaObject::invokeMethod(engine, slot, Qt::QueuedConnection, Q_ARG(int, generation));
    });
}

unsigned CallEngine::remoteProbe(GstPad *, void *, void *user)
{
    PipelineContext::of(user)->remotePackets.ref();
    return GST_PAD_PROBE_OK;
}

unsigned CallEngine::cameraProbe(GstPad *, void *, void *user)
{
    // Only the first few are worth reporting; after that the camera clearly
    // works and the log would drown.
    const int seen = PipelineContext::of(user)->cameraFrames.fetchAndAddOrdered(1) + 1;
    if (seen == 1 || seen == 30) {
        qInfo("xmatic: camera delivered %u frames", seen);
    }

    return GST_PAD_PROBE_OK;
}

int CallEngine::busMessage(void *, void *message, void *user)
{
    auto *msg = static_cast<GstMessage *>(message);
    auto *engine = static_cast<CallEngine *>(user);

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
        // Back to idle, like every other failure here: a dying pipeline used to
        // leave the call on "connecting" with no reason on screen.
        if (engine) {
            const QString reason = error && error->message
                    ? QString::fromUtf8(error->message)
                    : QString();
            QMetaObject::invokeMethod(engine, "reportPipelineFailure", Qt::QueuedConnection,
                                      Q_ARG(int, engine->m_generation.load()),
                                      Q_ARG(QString, reason));
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

void CallEngine::markDisconnected(int generation)
{
    if (!current(generation) || m_state == QLatin1String("idle")) {
        return;
    }
    qInfo("xmatic: call connection lost");
    logCandidates("failed");
    setStatus(tr("the connection was lost"));
    setFailure(tr("the connection was lost"));
    hangUp();
}

void CallEngine::markFailed(int generation)
{
    if (!current(generation) || m_state == QLatin1String("idle")) {
        return;
    }
    // The other side's candidates travel through the homeserver; on a phone
    // network they can land after libnice gave up, and new ones restart the checks.
    if (m_state == QLatin1String("connecting")) {
        if (!m_iceGrace.isActive()) {
            logCandidates("failed, waiting for late candidates");
            m_iceGrace.start();
        }
        return;
    }
    markDisconnected(generation);
}

void CallEngine::markConnected(int generation)
{
    if (!current(generation) || m_state == QLatin1String("idle")) {
        return;
    }
    m_iceGrace.stop();
    if (m_withVideo && cameraFrames() == 0) {
        qWarning("xmatic: connected but the camera has produced nothing");
    }

    logCandidates("connected");
    m_audioRouter.start();
    m_silentTicks = 0;
    m_lastRemotePackets = remotePackets();
    m_mediaWatch.start();
    setState(QStringLiteral("active"));
    setStatus(tr("connected"));
}

void CallEngine::deliverLocalDescription(int generation, const QString &sdp, bool isOffer)
{
    if (!current(generation)) {
        return;
    }
    qInfo("xmatic: local %s ready, %d bytes", isOffer ? "offer" : "answer", sdp.size());

    if (isOffer) {
        emit inviteReady(m_roomId, m_callId, m_partyId, sdp);
    } else {
        emit answerReady(m_roomId, m_callId, m_partyId, sdp);
    }
}

void CallEngine::deliverCandidate(int generation, const QString &candidate,
                                  int mediaLineIndex)
{
    if (!current(generation) || m_roomId.isEmpty() || m_callId.isEmpty()) {
        return;
    }

    ++m_localKinds[candidateKind(candidate)];
    QVariantMap entry;
    entry.insert(QStringLiteral("candidate"), candidate);
    entry.insert(QStringLiteral("sdpMLineIndex"), mediaLineIndex);

    // The caller's addresses wait for an answer: an unanswered call leaves none in
    // the room, and they cannot overtake the invite.
    if (m_isCaller && m_selectedParty.isEmpty()) {
        if (m_heldCandidates.size() < kHeldCandidates) {
            m_heldCandidates.append(entry);
        }
        return;
    }

    // Batched: ICE produces bursts, and one room event per candidate runs into the
    // rate limit - the tail is rejected and the call never connects.
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

/// A GStreamer error, queued so `reportFailure` never runs inside the bus
/// dispatch of the pipeline it tears down.
void CallEngine::reportPipelineFailure(int generation, const QString &reason)
{
    if (!current(generation)) {
        return;
    }
    reportFailure(reason.isEmpty() ? tr("the call could not be carried on")
                                   : tr("pipeline failed: %1").arg(reason));
}

void CallEngine::reportFailureFor(int generation, const QString &message)
{
    if (current(generation)) {
        reportFailure(message);
    }
}

void CallEngine::reportFailure(const QString &message)
{
    qWarning("xmatic: call engine: %s", qPrintable(message));
    setStatus(message);
    // The page pops itself on `idle`, so a reason set only here was never read.
    // Kept apart from the status and cleared when a call starts.
    setFailure(message);
    // The other side is waiting on a call this device has just given up on.
    // Every other way out of a call says so; this one said nothing.
    if (!m_roomId.isEmpty() && !m_callId.isEmpty()) {
        emit hangupReady(m_roomId, m_callId, m_partyId);
    }
    tearDown();
    m_earlyCandidates.clear();
    // Without this the last frame stays "active", and the page keeps drawing the
    // video layout - which hides both the reason and the way out.
    m_remoteVideo.stop();
    m_withVideo = false;
    setState(QStringLiteral("idle"));
}
