#ifndef APPEARANCESETTINGS_H
#define APPEARANCESETTINGS_H

#include <QObject>
#include <QVariant>
#include <QTimer>
#include <QHash>
#include <QString>

/// The user's colours: empty string means "follow the ambience". The opacities
/// apply to the bubble fills only - a half-transparent letter reads as broken.
class AppearanceSettings : public QObject
{
    Q_OBJECT

    Q_PROPERTY(QString ownBubbleColor READ ownBubbleColor WRITE setOwnBubbleColor NOTIFY changed)
    Q_PROPERTY(qreal ownBubbleOpacity READ ownBubbleOpacity WRITE setOwnBubbleOpacity NOTIFY changed)
    Q_PROPERTY(QString otherBubbleColor READ otherBubbleColor WRITE setOtherBubbleColor NOTIFY changed)
    Q_PROPERTY(qreal otherBubbleOpacity READ otherBubbleOpacity WRITE setOtherBubbleOpacity NOTIFY changed)
    Q_PROPERTY(QString nameColor READ nameColor WRITE setNameColor NOTIFY changed)
    Q_PROPERTY(QString ownTextColor READ ownTextColor WRITE setOwnTextColor NOTIFY changed)
    Q_PROPERTY(QString otherTextColor READ otherTextColor WRITE setOtherTextColor NOTIFY changed)

public:
    explicit AppearanceSettings(QObject *parent = nullptr);
    ~AppearanceSettings() override;

    QString ownBubbleColor() const { return m_ownBubbleColor; }
    qreal ownBubbleOpacity() const { return m_ownBubbleOpacity; }
    QString otherBubbleColor() const { return m_otherBubbleColor; }
    qreal otherBubbleOpacity() const { return m_otherBubbleOpacity; }
    QString nameColor() const { return m_nameColor; }
    QString ownTextColor() const { return m_ownTextColor; }
    QString otherTextColor() const { return m_otherTextColor; }

    void setOwnBubbleColor(const QString &color);
    void setOwnBubbleOpacity(qreal opacity);
    void setOtherBubbleColor(const QString &color);
    void setOtherBubbleOpacity(qreal opacity);
    void setNameColor(const QString &color);
    void setOwnTextColor(const QString &color);
    void setOtherTextColor(const QString &color);

    /// Back to the ambience for everything, one tap. The safety net for a
    /// palette someone talked themselves into and cannot read any more.
    Q_INVOKABLE void resetAll();

signals:
    void changed();

private:
    void load();
    void store(const QString &key, const QVariant &value);
    /// Writes what `store` collected; see the definition.
    void flush();
    QHash<QString, QVariant> m_unsaved;
    QTimer *m_writeTimer = nullptr;

    QString m_ownBubbleColor;
    qreal m_ownBubbleOpacity = 0.35;
    QString m_otherBubbleColor;
    qreal m_otherBubbleOpacity = 0.15;
    QString m_nameColor;
    QString m_ownTextColor;
    QString m_otherTextColor;
};

#endif // APPEARANCESETTINGS_H
