#ifndef CROSSSIGNING_H
#define CROSSSIGNING_H

#include <QJsonObject>
#include <QObject>
#include <QString>

/// `matrix.crossSigning`: a password account without an identity, and the one
/// password it is asked for. The password goes out once and is wiped.
class CrossSigning : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool needsPassword READ needsPassword NOTIFY changed)

public:
    explicit CrossSigning(QObject *parent = nullptr);

    bool needsPassword() const { return m_needsPassword; }

    Q_INVOKABLE void setUp(const QString &password);

    /// The core's `crossSigning.needsPassword`.
    void noteNeedsPassword();
    /// Answers to `crossSigning.setup`; `false` if the reply is not one.
    bool deliver(const QString &command, const QJsonObject &data);
    void reset();

signals:
    void changed();
    /// Sent with a wiped payload, like the sign-in.
    void commandReady(const QString &command, const QJsonObject &arguments);
    void refused(const QString &message);
    /// Another client had created the identity meanwhile; it was kept.
    void alreadyThere();

private:
    void setNeedsPassword(bool needs);

    bool m_needsPassword = false;
};

#endif // CROSSSIGNING_H
