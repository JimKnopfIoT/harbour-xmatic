#include "spacemarkers.h"

#include "appsettings.h"

#include <QColor>
#include <QCryptographicHash>
#include <QDir>
#include <QFileInfo>
#include <QJsonArray>
#include <QSettings>

namespace {

/// Where the chosen colours live inside the settings file.
const char *const GROUP = "spaceColours";

/// The letter drawn over a picture: the first character of the name, without a
/// sigil and without breaking a surrogate pair - an emoji is one letter, not two
/// halves of one.
QString initial(const QString &name)
{
    QString text = name;
    while (!text.isEmpty()
           && (text.at(0) == QLatin1Char('#') || text.at(0) == QLatin1Char('!')
               || text.at(0) == QLatin1Char('@') || text.at(0).isSpace())) {
        text.remove(0, 1);
    }
    if (text.isEmpty()) {
        return QString();
    }
    const int length = text.at(0).isHighSurrogate() && text.size() > 1 ? 2 : 1;
    return text.left(length).toUpper();
}

/// Whether the first space should carry the marker rather than the second. A
/// room can hang in several; the choice has to be the same after every restart.
bool preferred(const QString &name, const QString &id,
               const QString &heldName, const QString &heldId)
{
    const int byName = QString::compare(name, heldName, Qt::CaseInsensitive);
    return byName != 0 ? byName < 0 : id < heldId;
}

} // namespace

SpaceMarkers::SpaceMarkers(QObject *parent)
    : QObject(parent)
{
    loadColours();
}

void SpaceMarkers::update(const QJsonObject &spaces)
{
    m_parent.clear();
    m_name.clear();

    for (auto it = spaces.constBegin(); it != spaces.constEnd(); ++it) {
        const QJsonObject entry = it.value().toObject();
        const QString name = entry.value(QStringLiteral("name")).toString().trimmed();
        // A space no sync has named yet contributes no letter, and an identifier
        // is not a name - its sigil would say nothing.
        if (name.isEmpty()) {
            continue;
        }
        m_name.insert(it.key(), name);

        const QJsonArray rooms = entry.value(QStringLiteral("rooms")).toArray();
        for (const QJsonValue &value : rooms) {
            const QString roomId = value.toString();
            if (roomId.isEmpty()) {
                continue;
            }
            const QString held = m_parent.value(roomId);
            if (held.isEmpty()
                || preferred(name, it.key(), m_name.value(held), held)) {
                m_parent.insert(roomId, it.key());
            }
        }
    }

    ++m_revision;
    emit changed();
}

QVariantMap SpaceMarkers::markerFor(const QString &roomId) const
{
    return spaceMarker(m_parent.value(roomId));
}

QVariantMap SpaceMarkers::spaceMarker(const QString &spaceId) const
{
    if (spaceId.isEmpty()) {
        return QVariantMap();
    }
    const QString letter = initial(m_name.value(spaceId));
    if (letter.isEmpty()) {
        return QVariantMap();
    }

    const QString fill = spaceColour(spaceId);
    // The outline is held against the fill, not against the ambience: what has
    // to stay legible is one letter over somebody's photograph.
    const QColor colour(fill);
    const QString outline = colour.lightnessF() > 0.5 ? QStringLiteral("#000000")
                                                      : QStringLiteral("#ffffff");

    QVariantMap marker;
    marker.insert(QStringLiteral("letter"), letter);
    marker.insert(QStringLiteral("colour"), fill);
    marker.insert(QStringLiteral("outline"), outline);
    marker.insert(QStringLiteral("spaceId"), spaceId);
    marker.insert(QStringLiteral("spaceName"), m_name.value(spaceId));
    return marker;
}

QString SpaceMarkers::spaceColour(const QString &spaceId) const
{
    const QString chosen = m_chosen.value(colourKey(spaceId));
    return chosen.isEmpty() ? derivedColour(spaceId) : chosen;
}

bool SpaceMarkers::colourChosen(const QString &spaceId) const
{
    return m_chosen.contains(colourKey(spaceId));
}

void SpaceMarkers::setSpaceColour(const QString &spaceId, const QString &colour)
{
    const QColor wanted(colour);
    if (spaceId.isEmpty() || !wanted.isValid()) {
        return;
    }
    const QString value = wanted.name();
    const QString key = colourKey(spaceId);
    if (m_chosen.value(key) == value) {
        return;
    }
    m_chosen.insert(key, value);

    const QString path = appSettingsPath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSettings settings(path, QSettings::IniFormat);
    settings.setValue(QStringLiteral("%1/%2").arg(QLatin1String(GROUP), key), value);
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning("xmatic: could not save a space colour (status %d)",
                 static_cast<int>(settings.status()));
    }

    ++m_revision;
    emit changed();
}

void SpaceMarkers::resetSpaceColour(const QString &spaceId)
{
    const QString key = colourKey(spaceId);
    if (!m_chosen.contains(key)) {
        return;
    }
    m_chosen.remove(key);

    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    settings.remove(QStringLiteral("%1/%2").arg(QLatin1String(GROUP), key));
    settings.sync();

    ++m_revision;
    emit changed();
}

void SpaceMarkers::loadColours()
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    settings.beginGroup(QLatin1String(GROUP));
    const QStringList keys = settings.childKeys();
    for (const QString &key : keys) {
        const QColor colour(settings.value(key).toString());
        if (colour.isValid()) {
            m_chosen.insert(key, colour.name());
        }
    }
    settings.endGroup();
}

QString SpaceMarkers::colourKey(const QString &spaceId)
{
    // Digest, not identifier: the settings file is the one place a room id has
    // no business being, and an id is no good as an ini key either.
    return QString::fromLatin1(
        QCryptographicHash::hash(spaceId.toUtf8(), QCryptographicHash::Sha256)
            .toHex()
            .left(16));
}

QString SpaceMarkers::derivedColour(const QString &spaceId)
{
    const QByteArray digest =
        QCryptographicHash::hash(spaceId.toUtf8(), QCryptographicHash::Sha256);
    const qreal hue = static_cast<quint8>(digest.at(0)) / 256.0;
    // Saturation and lightness are fixed, only the hue varies: every space gets
    // a colour that carries over a dark and a light picture alike.
    return QColor::fromHsvF(hue, 0.55, 0.95).name();
}
