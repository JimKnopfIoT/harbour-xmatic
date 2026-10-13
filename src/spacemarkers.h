#ifndef SPACEMARKERS_H
#define SPACEMARKERS_H

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QVariantMap>

/// The space a room belongs to, as one initial drawn over its picture:
/// `matrix.spaceMarkers`. Owns the room-to-space map, the colour per space and
/// nothing else - the badge counts stay in the bridge.
class SpaceMarkers : public QObject
{
    Q_OBJECT

    /// Bumped on every change to the map or a colour, so a delegate's binding
    /// re-evaluates. The map arrives as a whole, not as a diff.
    Q_PROPERTY(int revision READ revision NOTIFY changed)
    /// False while the encrypted lists cannot be read: a choice could not be kept.
    Q_PROPERTY(bool editable READ editable NOTIFY changed)

public:
    explicit SpaceMarkers(QObject *parent = nullptr);

    /// The `spaces.children` payload: space id to rooms, subspace count and name.
    void update(const QJsonObject &spaces);

    /// Letter, fill and outline for a room, or an empty map where the room is
    /// in no named space. One call per row: the page asks nothing else.
    Q_INVOKABLE QVariantMap markerFor(const QString &roomId) const;

    /// The same for a space itself, which is what the colour page previews.
    Q_INVOKABLE QVariantMap spaceMarker(const QString &spaceId) const;

    /// The colour a space is marked in - the user's choice where there is one,
    /// otherwise derived from the space's identifier.
    Q_INVOKABLE QString spaceColour(const QString &spaceId) const;
    /// Whether that colour was chosen rather than derived.
    Q_INVOKABLE bool colourChosen(const QString &spaceId) const;
    Q_INVOKABLE void setSpaceColour(const QString &spaceId, const QString &colour);
    /// Back to the derived colour.
    Q_INVOKABLE void resetSpaceColour(const QString &spaceId);

    int revision() const { return m_revision; }
    bool editable() const { return m_editable; }
    void setEditable(bool editable);

    /// The chosen colours as `digest=#rrggbb`, kept in the encrypted lists.
    QStringList storedEntries() const;
    void adoptStored(const QStringList &entries);
    /// Colours an older version kept in the plain settings file, once.
    QStringList legacyEntries() const;
    void dropLegacy();
    /// A new account starts with derived colours.
    void clear();

signals:
    void changed();
    /// A colour was chosen or reset; the owner writes the encrypted copy.
    void coloursEdited();
    /// A change asked for while the lists are not editable.
    void editRefused();

private:
    static QString colourKey(const QString &spaceId);
    /// A colour from the identifier alone: same space, same colour, every start.
    static QString derivedColour(const QString &spaceId);

    QHash<QString, QString> m_parent;
    QHash<QString, QString> m_name;
    QHash<QString, QString> m_chosen;
    int m_revision = 0;
    bool m_editable = false;
};

#endif // SPACEMARKERS_H
