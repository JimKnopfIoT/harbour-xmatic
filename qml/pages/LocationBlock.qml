import QtQuick 2.0
import Sailfish.Silica 1.0

// A location in a bubble: a map where the setting allows one, the coordinates
// always. The width comes from the caller; nothing here measures the parent.
Column {
    id: block

    /// The row's `location` field, as the core built it.
    property var location

    /// Set by the caller; the children read this item's width.
    property real availableWidth: 0

    /// Unknown counts as encrypted: the map gate's safe direction.
    property bool encrypted: true

    property bool own: false
    // Drawn at the point where the location is the sender's own.
    property string avatarSource: ""
    property string avatarName: ""
    property string roomId

    readonly property bool hasPoint: !!location && typeof location.lat === "number"
                                     && typeof location.lon === "number"
    readonly property bool live: !!location && location.live === true
    readonly property bool ownPosition: !!location && location.self === true
    /// The SDK's reading at row time, held against the clock: no diff marks the expiry.
    property real now: Date.now()
    readonly property bool active: live && location.active === true && now < location.until

    property var mapData: ({})
    readonly property bool hasMap: hasPoint && !!mapData && mapData.available === true
    readonly property string mapKey: hasPoint
                                     ? matrix.locations.mapKey(location.lat, location.lon) : ""
    readonly property real mapScale: hasMap ? width / mapData.width : 1

    /// `map`: the card's map for the confirmation page, or null.
    signal activated(string link, var map)

    width: availableWidth
    spacing: Theme.paddingSmall

    function loadMap() {
        if (!block.hasPoint) {
            block.mapData = ({})
            return
        }
        block.mapData = matrix.locations.map(location.lat, location.lon, block.encrypted)
    }

    Component.onCompleted: loadMap()
    onMapKeyChanged: loadMap()
    onEncryptedChanged: loadMap()

    Connections {
        target: matrix.locations
        onMapReady: {
            if (key === block.mapKey) {
                block.mapData = map
            }
        }
    }

    Connections {
        target: settings
        onLocationMapsChanged: block.loadMap()
    }

    Timer {
        interval: 30000
        repeat: true
        running: block.live && block.active
        onTriggered: block.now = Date.now()
    }

    function osmLink() {
        var lat = location.lat.toFixed(5)
        var lon = location.lon.toFixed(5)
        return "https://www.openstreetmap.org/?mlat=" + lat + "&mlon=" + lon
                + "#map=16/" + lat + "/" + lon
    }

    function timeText(ms) {
        return Format.formatDate(new Date(ms), Formatter.TimeValue)
    }

    Item {
        width: block.width
        height: block.hasMap ? Math.round(block.width * block.mapData.height / block.mapData.width) : 0
        visible: block.hasMap
        clip: true

        Repeater {
            model: block.hasMap ? block.mapData.tiles : []

            Image {
                x: modelData.left * block.mapScale
                y: modelData.top * block.mapScale
                width: block.mapData.tile * block.mapScale
                height: width
                asynchronous: true
                smooth: true
                sourceSize.width: block.mapData.tile
                sourceSize.height: block.mapData.tile
                source: "file://" + modelData.path
            }
        }

        // The point, where the core centred it.
        Rectangle {
            visible: !block.ownPosition
            width: Theme.paddingLarge
            height: width
            radius: width / 2
            anchors.centerIn: parent
            color: block.live && !block.active ? Theme.secondaryColor : Theme.highlightColor
            border.color: "white"
            border.width: Math.max(1, Math.round(Theme.paddingSmall / 3))
        }

        // The sender's own position: their picture, ringed.
        Rectangle {
            visible: block.ownPosition
            width: selfAvatar.size + 2 * border.width
            height: width
            radius: width / 2
            anchors.centerIn: parent
            color: "white"
            border.color: block.live && !block.active ? Theme.secondaryColor : Theme.highlightColor
            border.width: Math.max(2, Math.round(Theme.paddingSmall / 2))
            opacity: block.live && !block.active ? 0.6 : 1

            Avatar {
                id: selfAvatar

                anchors.centerIn: parent
                size: Theme.iconSizeMedium
                source: block.avatarSource
                name: block.avatarName
            }
        }

        // Required by the tile licence, and it says who drew the map.
        Rectangle {
            anchors {
                right: parent.right
                bottom: parent.bottom
            }
            width: attribution.width + Theme.paddingSmall
            height: attribution.height
            color: Qt.rgba(1, 1, 1, 0.7)

            Label {
                id: attribution

                anchors.centerIn: parent
                font.pixelSize: Theme.fontSizeTiny
                color: "black"
                textFormat: Text.PlainText
                text: "© OpenStreetMap contributors"
            }
        }

        MouseArea {
            anchors.fill: parent
            onClicked: block.activated(block.osmLink(), block.mapData)
        }
    }

    BackgroundItem {
        width: block.width
        height: texts.height + Theme.paddingSmall
        enabled: block.hasPoint
        onClicked: block.activated(block.osmLink(), block.hasMap ? block.mapData : null)

        Column {
            id: texts

            width: parent.width

            Label {
                width: parent.width
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeSmall
                color: block.active ? Theme.highlightColor : Theme.primaryColor
                text: !block.live ? qsTr("Location")
                                  : (block.active ? qsTr("Live location")
                                                  : qsTr("Live location ended"))
            }

            Label {
                width: parent.width
                visible: block.hasPoint
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: block.hasPoint
                      ? block.location.lat.toFixed(5) + ", " + block.location.lon.toFixed(5)
                        + (typeof block.location.accuracy === "number"
                           ? " · ±" + Math.round(block.location.accuracy) + " m" : "")
                      : ""
            }

            Label {
                width: parent.width
                visible: block.live && (block.active || !block.hasPoint)
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: {
                    if (!block.live) {
                        return ""
                    }
                    if (!block.hasPoint) {
                        return block.active ? qsTr("Waiting for the first position") : ""
                    }
                    return qsTr("Updated %1, until %2")
                            .arg(block.timeText(block.location.updated))
                            .arg(block.timeText(block.location.until))
                }
            }

            Label {
                width: parent.width
                visible: !!block.location && !!block.location.description
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: block.location && block.location.description ? block.location.description : ""
            }
        }
    }

    // Also for a share left over from before a restart: the row knows it, this run does not.
    WrapButton {
        anchors.horizontalCenter: parent.horizontalCenter
        width: Math.min(Theme.buttonWidthMedium, block.width)
        visible: block.own && block.active
        label: qsTr("Stop sharing")
        onClicked: matrix.locations.stopLive(block.roomId)
    }
}
