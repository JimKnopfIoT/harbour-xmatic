#include "callaudiorouter.h"

#include <QTimer>

#include <cstring>
#include <pulse/pulseaudio.h>

namespace {

// The name the call's playback stream carries, distinct from the app's own so
// only call audio is matched, not a notification sound.
const char *kCallStreamName = "xmatic call";

void portSetCb(pa_context *, int success, void *userdata)
{
    qInfo("xmatic: audio port %s %s", static_cast<const char *>(userdata), success ? "set" : "refused");
}

void contextStateCb(pa_context *, void *userdata)
{
    pa_threaded_mainloop_signal(static_cast<pa_threaded_mainloop *>(userdata), 0);
}

// The real output sink is the one with both a speaker and an earpiece port:
// that skips sink.null and the deep-buffer sink, and stays device-agnostic.
struct SinkScan {
    pa_threaded_mainloop *ml = nullptr;
    QString sink;
    QString speaker;
    QString earpiece;
};

void sinkInfoCb(pa_context *, const pa_sink_info *info, int eol, void *userdata)
{
    auto *scan = static_cast<SinkScan *>(userdata);
    if (eol) {
        pa_threaded_mainloop_signal(scan->ml, 0);
        return;
    }
    if (!info || !scan->sink.isEmpty()) {
        return;
    }
    QString speaker;
    QString earpiece;
    for (uint32_t p = 0; p < info->n_ports; ++p) {
        const pa_sink_port_info *port = info->ports[p];
        if (!port || !port->name) {
            continue;
        }
        const QString name = QString::fromUtf8(port->name).toLower();
        if (speaker.isEmpty() && name.contains(QLatin1String("speaker"))) {
            speaker = QString::fromUtf8(port->name);
        }
        if (earpiece.isEmpty()
            && (name.contains(QLatin1String("earpiece")) || name.contains(QLatin1String("handset"))
                || name.contains(QLatin1String("receiver")))) {
            earpiece = QString::fromUtf8(port->name);
        }
    }
    if (!speaker.isEmpty() && !earpiece.isEmpty()) {
        scan->sink = QString::fromUtf8(info->name);
        scan->speaker = speaker;
        scan->earpiece = earpiece;
    }
}

struct InputScan {
    pa_threaded_mainloop *ml = nullptr;
    uint32_t index = PA_INVALID_INDEX;
    uint8_t channels = 2;
    bool found = false;
};

void sinkInputInfoCb(pa_context *, const pa_sink_input_info *info, int eol, void *userdata)
{
    auto *scan = static_cast<InputScan *>(userdata);
    if (eol) {
        pa_threaded_mainloop_signal(scan->ml, 0);
        return;
    }
    if (!info || !info->proplist) {
        return;
    }
    const char *app = pa_proplist_gets(info->proplist, "application.name");
    if (app && std::strstr(app, kCallStreamName)) {
        scan->index = info->index;
        scan->channels = info->volume.channels > 0 ? info->volume.channels : 2;
        scan->found = true;
    }
}

} // namespace

CallAudioRouter::CallAudioRouter(QObject *parent)
    : QObject(parent)
    , m_retryTimer(new QTimer(this))
{
    m_retryTimer->setInterval(500);
    connect(m_retryTimer, &QTimer::timeout, this, [this]() {
        if (routeCallStream()) {
            m_routed = true;
            m_retryTimer->stop();
        }
    });
}

CallAudioRouter::~CallAudioRouter()
{
    stop();
    // Under the loop's lock: its thread dispatches into the context until then.
    if (m_context && m_mainloop) {
        auto *ml = static_cast<pa_threaded_mainloop *>(m_mainloop);
        auto *ctx = static_cast<pa_context *>(m_context);
        pa_threaded_mainloop_lock(ml);
        pa_context_set_state_callback(ctx, nullptr, nullptr);
        pa_context_disconnect(ctx);
        pa_context_unref(ctx);
        pa_threaded_mainloop_unlock(ml);
        m_context = nullptr;
    }
    if (m_mainloop) {
        pa_threaded_mainloop_stop(static_cast<pa_threaded_mainloop *>(m_mainloop));
        pa_threaded_mainloop_free(static_cast<pa_threaded_mainloop *>(m_mainloop));
        m_mainloop = nullptr;
    }
}

void CallAudioRouter::ensureConnection()
{
    if (m_context) {
        auto *ml = static_cast<pa_threaded_mainloop *>(m_mainloop);
        auto *ctx = static_cast<pa_context *>(m_context);
        pa_threaded_mainloop_lock(ml);
        const pa_context_state_t st = pa_context_get_state(ctx);
        if (st != PA_CONTEXT_FAILED && st != PA_CONTEXT_TERMINATED) {
            pa_threaded_mainloop_unlock(ml);
            return;
        }
        // The sound server restarted: a dead context never comes back.
        pa_context_set_state_callback(ctx, nullptr, nullptr);
        pa_context_unref(ctx);
        pa_threaded_mainloop_unlock(ml);
        pa_threaded_mainloop_stop(ml);
        pa_threaded_mainloop_free(ml);
        m_context = nullptr;
        m_mainloop = nullptr;
    }
    pa_threaded_mainloop *ml = pa_threaded_mainloop_new();
    if (!ml) {
        return;
    }
    pa_threaded_mainloop_start(ml);
    pa_threaded_mainloop_lock(ml);

    pa_context *ctx = pa_context_new(pa_threaded_mainloop_get_api(ml), "xmatic");
    pa_context_set_state_callback(ctx, &contextStateCb, ml);
    pa_context_connect(ctx, nullptr, PA_CONTEXT_NOFLAGS, nullptr);
    for (;;) {
        const pa_context_state_t st = pa_context_get_state(ctx);
        if (st == PA_CONTEXT_READY) {
            break;
        }
        if (st == PA_CONTEXT_FAILED || st == PA_CONTEXT_TERMINATED) {
            pa_threaded_mainloop_unlock(ml);
            pa_context_unref(ctx);
            pa_threaded_mainloop_stop(ml);
            pa_threaded_mainloop_free(ml);
            return;
        }
        pa_threaded_mainloop_wait(ml);
    }

    SinkScan scan;
    scan.ml = ml;
    pa_operation *op = pa_context_get_sink_info_list(ctx, &sinkInfoCb, &scan);
    if (op) {
        while (pa_operation_get_state(op) == PA_OPERATION_RUNNING) {
            pa_threaded_mainloop_wait(ml);
        }
        pa_operation_unref(op);
    }
    pa_threaded_mainloop_unlock(ml);

    m_mainloop = ml;
    m_context = ctx;
    m_sink = scan.sink;
    m_speakerPort = scan.speaker;
    m_earpiecePort = scan.earpiece;
    if (m_sink.isEmpty()) {
        qWarning("xmatic: audio router found no sink with speaker and earpiece");
    } else {
        qInfo("xmatic: audio router uses %s (%s, %s)", qPrintable(m_sink),
              qPrintable(m_speakerPort), qPrintable(m_earpiecePort));
    }
}

bool CallAudioRouter::routeCallStream()
{
    ensureConnection();
    auto *ctx = static_cast<pa_context *>(m_context);
    auto *ml = static_cast<pa_threaded_mainloop *>(m_mainloop);
    if (!ctx || !ml) {
        return false;
    }

    InputScan scan;
    scan.ml = ml;
    pa_threaded_mainloop_lock(ml);
    if (pa_context_get_state(ctx) != PA_CONTEXT_READY) {
        pa_threaded_mainloop_unlock(ml);
        qWarning("xmatic: audio router lost its sound server connection");
        return false;
    }
    pa_operation *op = pa_context_get_sink_input_info_list(ctx, &sinkInputInfoCb, &scan);
    if (op) {
        while (pa_operation_get_state(op) == PA_OPERATION_RUNNING) {
            pa_threaded_mainloop_wait(ml);
        }
        pa_operation_unref(op);
    }

    if (scan.found) {
        // Move onto the real output sink, unmute (born muted on Halium), and
        // set a sane volume in case the stream came up at zero.
        if (!m_sink.isEmpty()) {
            if (pa_operation *o = pa_context_move_sink_input_by_name(
                    ctx, scan.index, m_sink.toUtf8().constData(), nullptr, nullptr)) {
                pa_operation_unref(o);
            }
        }
        if (pa_operation *o =
                pa_context_set_sink_input_mute(ctx, scan.index, 0, nullptr, nullptr)) {
            pa_operation_unref(o);
        }
        // Half, not nine tenths: the stream is born muted on Halium and needs a value,
        // and a call at full volume against an ear is a fright.
        pa_cvolume cv;
        pa_cvolume_set(&cv, scan.channels, PA_VOLUME_NORM / 2);
        if (pa_operation *o =
                pa_context_set_sink_input_volume(ctx, scan.index, &cv, nullptr, nullptr)) {
            pa_operation_unref(o);
        }
        qInfo("xmatic: call stream %u moved to %s", scan.index, qPrintable(m_sink));
        // Setting the port also wakes the sink, so a moved-in stream plays.
        applyPort(m_speakerOn);
    }

    pa_threaded_mainloop_unlock(ml);
    return scan.found;
}

void CallAudioRouter::setSpeaker(bool on)
{
    m_speakerOn = on;
    auto *ml = static_cast<pa_threaded_mainloop *>(m_mainloop);
    if (!ml || (!m_retryTimer->isActive() && !m_routed)) {
        qInfo("xmatic: %s kept for when the call is routed", on ? "speaker" : "earpiece");
        return;
    }
    pa_threaded_mainloop_lock(ml);
    applyPort(on);
    pa_threaded_mainloop_unlock(ml);
}

// Under the loop's lock.
void CallAudioRouter::applyPort(bool speaker)
{
    auto *ctx = static_cast<pa_context *>(m_context);
    const QString &port = speaker ? m_speakerPort : m_earpiecePort;
    if (!ctx || m_sink.isEmpty() || port.isEmpty()
        || pa_context_get_state(ctx) != PA_CONTEXT_READY) {
        return;
    }
    // The callback only names the port: these two strings outlive every call.
    const char *label = speaker ? "speaker" : "earpiece";
    if (pa_operation *o = pa_context_set_sink_port_by_name(
            ctx, m_sink.toUtf8().constData(), port.toUtf8().constData(), &portSetCb,
            const_cast<char *>(label))) {
        pa_operation_unref(o);
    }
}

void CallAudioRouter::start()
{
    m_routed = false;
    ensureConnection();
    m_routed = routeCallStream();
    m_retryTimer->start();
}

void CallAudioRouter::stop()
{
    m_retryTimer->stop();
    // The sink is the system's: leave it on the loudspeaker, as found.
    auto *ml = static_cast<pa_threaded_mainloop *>(m_mainloop);
    if (ml && m_routed && !m_speakerOn) {
        pa_threaded_mainloop_lock(ml);
        applyPort(true);
        pa_threaded_mainloop_unlock(ml);
    }
    m_routed = false;
}
