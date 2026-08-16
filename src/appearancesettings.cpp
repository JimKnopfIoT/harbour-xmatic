#include "appearancesettings.h"

#include <QDir>
#include <QFileInfo>
#include <QSettings>
#include <QStandardPaths>
#include <QVariant>

// Same reasoning as the bridge's settings: Sailjail only lets the app write
// inside its own config directory, QSettings' UserScope default sits one
// level above it and the sandbox blocks the write silently. Same file too —
// one settings.conf, this class owns the "appearance" group.
static QString settingsPath()
{
    return QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation)
           + QStringLiteral("/settings.conf");
}

// The bubble fills' defaults, kept in one place: 0.35 marks the own side,
// 0.15 the received one — the soft tints the contrast work settled on.
static const qreal kOwnOpacityDefault = 0.35;
static const qreal kOtherOpacityDefault = 0.15;

AppearanceSettings::AppearanceSettings(QObject *parent)
    : QObject(parent)
{
    load();
}

void AppearanceSettings::load()
{
    QSettings settings(settingsPath(), QSettings::IniFormat);
    settings.beginGroup(QStringLiteral("appearance"));
    m_ownBubbleColor = settings.value(QStringLiteral("ownBubbleColor")).toString();
    m_ownBubbleOpacity =
        settings.value(QStringLiteral("ownBubbleOpacity"), kOwnOpacityDefault).toReal();
    m_otherBubbleColor = settings.value(QStringLiteral("otherBubbleColor")).toString();
    m_otherBubbleOpacity =
        settings.value(QStringLiteral("otherBubbleOpacity"), kOtherOpacityDefault).toReal();
    m_nameColor = settings.value(QStringLiteral("nameColor")).toString();
    m_ownTextColor = settings.value(QStringLiteral("ownTextColor")).toString();
    m_otherTextColor = settings.value(QStringLiteral("otherTextColor")).toString();
}

void AppearanceSettings::store(const QString &key, const QVariant &value)
{
    const QString path = settingsPath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSettings settings(path, QSettings::IniFormat);
    settings.setValue(QStringLiteral("appearance/") + key, value);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not save an appearance setting (status %d)",
                 static_cast<int>(settings.status()));
    }
}

void AppearanceSettings::setOwnBubbleColor(const QString &color)
{
    if (color == m_ownBubbleColor) {
        return;
    }
    m_ownBubbleColor = color;
    store(QStringLiteral("ownBubbleColor"), color);
    emit changed();
}

void AppearanceSettings::setOwnBubbleOpacity(qreal opacity)
{
    if (qFuzzyCompare(opacity, m_ownBubbleOpacity)) {
        return;
    }
    m_ownBubbleOpacity = opacity;
    store(QStringLiteral("ownBubbleOpacity"), opacity);
    emit changed();
}

void AppearanceSettings::setOtherBubbleColor(const QString &color)
{
    if (color == m_otherBubbleColor) {
        return;
    }
    m_otherBubbleColor = color;
    store(QStringLiteral("otherBubbleColor"), color);
    emit changed();
}

void AppearanceSettings::setOtherBubbleOpacity(qreal opacity)
{
    if (qFuzzyCompare(opacity, m_otherBubbleOpacity)) {
        return;
    }
    m_otherBubbleOpacity = opacity;
    store(QStringLiteral("otherBubbleOpacity"), opacity);
    emit changed();
}

void AppearanceSettings::setNameColor(const QString &color)
{
    if (color == m_nameColor) {
        return;
    }
    m_nameColor = color;
    store(QStringLiteral("nameColor"), color);
    emit changed();
}

void AppearanceSettings::setOwnTextColor(const QString &color)
{
    if (color == m_ownTextColor) {
        return;
    }
    m_ownTextColor = color;
    store(QStringLiteral("ownTextColor"), color);
    emit changed();
}

void AppearanceSettings::setOtherTextColor(const QString &color)
{
    if (color == m_otherTextColor) {
        return;
    }
    m_otherTextColor = color;
    store(QStringLiteral("otherTextColor"), color);
    emit changed();
}

void AppearanceSettings::resetAll()
{
    m_ownBubbleColor.clear();
    m_ownBubbleOpacity = kOwnOpacityDefault;
    m_otherBubbleColor.clear();
    m_otherBubbleOpacity = kOtherOpacityDefault;
    m_nameColor.clear();
    m_ownTextColor.clear();
    m_otherTextColor.clear();

    const QString path = settingsPath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSettings settings(path, QSettings::IniFormat);
    settings.remove(QStringLiteral("appearance"));
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not reset the appearance settings (status %d)",
                 static_cast<int>(settings.status()));
    }
    emit changed();
}
