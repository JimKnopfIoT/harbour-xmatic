#ifndef VOICETRANSCRIPTS_H
#define VOICETRANSCRIPTS_H

#include <QHash>
#include <QList>
#include <QObject>
#include <QPair>
#include <QSet>
#include <QString>
#include <QStringList>
#include <QTimer>
#include <QVariant>

#include <functional>

class QDBusMessage;
class QProcess;

/// Voice messages to text, on request only: `matrix.transcripts`. Decodes in a
/// helper inside the jail, recognises through the offline speech service,
/// keeps the text in memory and nowhere else.
class VoiceTranscripts : public QObject
{
    Q_OBJECT

    /// Bumped on every change, so bindings on state() and text() read again.
    Q_PROPERTY(int revision READ revision NOTIFY revisionChanged)
    /// "", "running", "ok" or "failed", and what the last check found.
    Q_PROPERTY(QString checkState READ checkState NOTIFY checkChanged)
    Q_PROPERTY(QString checkMessage READ checkMessage NOTIFY checkChanged)

public:
    explicit VoiceTranscripts(const QString &workDirectory, QObject *parent = nullptr);

    int revision() const { return m_revision; }
    QString checkState() const { return m_checkState; }
    QString checkMessage() const { return m_checkMessage; }

    /// Converts one voice message. `itemId` keys the download, `eventId` the result.
    Q_INVOKABLE void transcribe(const QString &itemId, const QString &eventId,
                                const QVariant &media);
    /// "", "working", "done" or "failed".
    Q_INVOKABLE QString state(const QString &eventId) const;
    Q_INVOKABLE QString text(const QString &eventId) const;
    Q_INVOKABLE QString failure(const QString &eventId) const;

    /// The whole path once, with a bundled second of silence.
    Q_INVOKABLE void check();

    /// Forgets every transcript and stops what runs. For signing out.
    void clear();

public slots:
    void mediaArrived(const QString &key, const QString &path);

signals:
    void revisionChanged();
    void checkChanged();
    /// Fetched through the page's own download path; the bridge wires this up.
    void mediaWanted(const QString &key, const QVariant &source, qint64 declaredSize);

private slots:
    void onTextDecoded(const QString &text, const QString &lang, int task);
    void onTranscribeFinished(int task);
    void onServiceError(int code);

private:
    struct Job {
        QString eventId;
        QString mediaKey;
        QVariant source;
        qint64 declaredSize = 0;
        QString mediaPath;
        QString wavPath;
        QString owner;
        QString model;
        QStringList pieces;
        int task = -1;
        bool check = false;
    };

    void markFailed(const QString &eventId, const QString &reason);
    void bump();
    void next();
    void decode();
    void decoded(int exitCode, bool crashed);
    void activate();
    void verify();
    void waitIdle(int attemptsLeft);
    void startTranscription();
    void keepAlive();
    void finish();
    void fail(const QString &reason);
    void endJob();
    void arm(int milliseconds, const QString &reason);
    void setCheck(const QString &state, const QString &message);
    void subscribe(bool on);
    QString serviceProblem(const QDBusMessage &reply) const;
    void ask(const QString &destination, const QString &path, const QString &interface,
             const QString &method, const QVariantList &arguments,
             std::function<void(const QDBusMessage &)> done);

    QString m_workDirectory;
    QList<Job> m_queue;
    Job m_job;
    bool m_busy = false;
    /// Advanced per job and per end: an answer that arrives later is ignored.
    quint64 m_generation = 0;
    QProcess *m_helper = nullptr;
    QString m_subscribed;
    QTimer m_keepAlive;
    QTimer m_deadline;
    QString m_deadlineReason;
    /// Signals can overtake the reply that names the task.
    QList<QPair<int, QString>> m_early;
    QSet<int> m_earlyDone;

    QHash<QString, QString> m_states;
    QHash<QString, QString> m_texts;
    QHash<QString, QString> m_failures;
    int m_revision = 0;
    QString m_checkState;
    QString m_checkMessage;
};

#endif // VOICETRANSCRIPTS_H
