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

void SpaceMarkers::setEditable(bool editable)
{
    if (m_editable == editable) {
        return;
    }
    m_editable = editable;
    ++m_revision;
    emit changed();
}

void SpaceMarkers::setSpaceColour(const QString &spaceId, const QString &colour)
{
    if (!m_editable) {
        emit editRefused();
        return;
    }
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

    ++m_revision;
    emit changed();
    emit coloursEdited();
}

void SpaceMarkers::resetSpaceColour(const QString &spaceId)
{
    if (!m_editable) {
        emit editRefused();
        return;
    }
    const QString key = colourKey(spaceId);
    if (!m_chosen.contains(key)) {
        return;
    }
    m_chosen.remove(key);

    ++m_revision;
    emit changed();
    emit coloursEdited();
}

QStringList SpaceMarkers::storedEntries() const
{
    QStringList entries;
    for (auto it = m_chosen.constBegin(); it != m_chosen.constEnd(); ++it) {
        entries.append(it.key() + QLatin1Char('=') + it.value());
    }
    entries.sort();
    return entries;
}

void SpaceMarkers::adoptStored(const QStringList &entries)
{
    QHash<QString, QString> chosen;
    for (const QString &entry : entries) {
        const int split = entry.indexOf(QLatin1Char('='));
        const QColor colour(entry.mid(split + 1));
        if (split > 0 && colour.isValid()) {
            chosen.insert(entry.left(split), colour.name());
        }
    }
    if (chosen == m_chosen) {
        return;
    }
    m_chosen = chosen;
    ++m_revision;
    emit changed();
}

QStringList SpaceMarkers::legacyEntries() const
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    settings.beginGroup(QLatin1String(GROUP));
    QStringList entries;
    for (const QString &key : settings.childKeys()) {
        const QColor colour(settings.value(key).toString());
        if (colour.isValid()) {
            entries.append(key + QLatin1Char('=') + colour.name());
        }
    }
    return entries;
}

void SpaceMarkers::dropLegacy()
{
    QSettings settings(appSettingsPath(), QSettings::IniFormat);
    settings.remove(QLatin1String(GROUP));
    settings.sync();
}

void SpaceMarkers::clear()
{
    if (m_chosen.isEmpty()) {
        return;
    }
    m_chosen.clear();
    ++m_revision;
    emit changed();
}

QString SpaceMarkers::colourKey(const QString &spaceId)
{
    // Digest, not identifier: even inside the encrypted lists a room id is
    // more than a colour needs.
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
