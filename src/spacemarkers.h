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

signals:
    void changed();

private:
    /// The stored colours, keyed by the digest below: the settings file never
    /// carries a room identifier.
    void loadColours();
    static QString colourKey(const QString &spaceId);
    /// A colour from the identifier alone: same space, same colour, every start.
    static QString derivedColour(const QString &spaceId);

    QHash<QString, QString> m_parent;
    QHash<QString, QString> m_name;
    QHash<QString, QString> m_chosen;
    int m_revision = 0;
};

#endif // SPACEMARKERS_H
