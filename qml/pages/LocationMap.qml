import QtQuick 2.0
import Sailfish.Silica 1.0

// The card's cached tiles, filling the area the caller gives it, zoomable.
// Fetches nothing: zoom scales what the card already has. Size from the caller.
Item {
    id: map

    /// The card's `mapData`, as `location.tiles` answered it.
    property var mapData: ({})

    readonly property var tiles: !!mapData && !!mapData.tiles ? mapData.tiles : []
    readonly property real tileSize: !!mapData && mapData.tile > 0 ? mapData.tile : 256

    /// The tiles' extent in map pixels, relative to the card's view.
    readonly property var bounds: {
        var left = 0, top = 0, right = 0, bottom = 0
        for (var i = 0; i < tiles.length; ++i) {
            var tile = tiles[i]
            if (i === 0 || tile.left < left) left = tile.left
            if (i === 0 || tile.top < top) top = tile.top
            if (i === 0 || tile.left + tileSize > right) right = tile.left + tileSize
            if (i === 0 || tile.top + tileSize > bottom) bottom = tile.top + tileSize
        }
        return { "left": left, "top": top, "width": right - left, "height": bottom - top }
    }
    /// Covers the area; the overhang is cropped.
    readonly property real fit: bounds.width > 0 && bounds.height > 0
                                ? Math.max(width / bounds.width, height / bounds.height) : 0
    /// Screen pixels per map pixel at most: street names finger-high, soft.
    readonly property real maxUnit: 6 * Theme.pixelRatio
    readonly property real maxZoom: fit > 0 ? Math.max(1, maxUnit / fit) : 1

    /// The point, in map pixels from the tiles' corner.
    readonly property real pointX: !!mapData ? mapData.width / 2 - bounds.left : 0
    readonly property real pointY: !!mapData ? mapData.height / 2 - bounds.top : 0

    visible: tiles.length > 0
    clip: true
    onFitChanged: if (!flick.touched) flick.centre()

    Flickable {
        id: flick

        readonly property real zoom: map.fit > 0 && map.bounds.width > 0
                                     ? contentWidth / (map.bounds.width * map.fit) : 1
        readonly property real unit: map.fit * zoom
        property bool touched: false

        function zoomTo(factor, center) {
            factor = Math.max(1, Math.min(map.maxZoom, factor))
            resizeContent(map.bounds.width * map.fit * factor,
                          map.bounds.height * map.fit * factor, center)
            returnToBounds()
        }

        /// Point in the middle, as far as the edges allow.
        function centre() {
            contentWidth = map.bounds.width * map.fit
            contentHeight = map.bounds.height * map.fit
            contentX = Math.max(0, Math.min(contentWidth - width, map.pointX * unit - width / 2))
            contentY = Math.max(0, Math.min(contentHeight - height, map.pointY * unit - height / 2))
        }

        anchors.fill: parent
        interactive: contentWidth > width + 1 || contentHeight > height + 1
        boundsBehavior: Flickable.StopAtBounds
        onMovementStarted: touched = true

        // Until the first touch the size may still settle; the point stays centred.
        onWidthChanged: if (!touched) centre()
        onHeightChanged: if (!touched) centre()
        Component.onCompleted: centre()

        PinchArea {
            property real startZoom: 1

            width: Math.max(flick.contentWidth, flick.width)
            height: Math.max(flick.contentHeight, flick.height)

            onPinchStarted: {
                flick.touched = true
                startZoom = flick.zoom
            }
            onPinchUpdated: {
                flick.contentX += pinch.previousCenter.x - pinch.center.x
                flick.contentY += pinch.previousCenter.y - pinch.center.y
                var factor = Math.max(1, Math.min(map.maxZoom, startZoom * pinch.scale))
                flick.resizeContent(map.bounds.width * map.fit * factor,
                                    map.bounds.height * map.fit * factor, pinch.center)
            }
            onPinchFinished: flick.returnToBounds()

            Item {
                width: flick.contentWidth
                height: flick.contentHeight

                Repeater {
                    model: map.tiles

                    Image {
                        x: (modelData.left - map.bounds.left) * flick.unit
                        y: (modelData.top - map.bounds.top) * flick.unit
                        width: map.tileSize * flick.unit
                        height: width
                        asynchronous: true
                        smooth: true
                        sourceSize.width: map.tileSize
                        sourceSize.height: map.tileSize
                        source: "file://" + modelData.path
                    }
                }

                Rectangle {
                    x: map.pointX * flick.unit - width / 2
                    y: map.pointY * flick.unit - height / 2
                    width: Theme.paddingLarge
                    height: width
                    radius: width / 2
                    color: Theme.highlightColor
                    border.color: "white"
                    border.width: Math.max(1, Math.round(Theme.paddingSmall / 3))
                }

                MouseArea {
                    anchors.fill: parent
                    onDoubleClicked: {
                        flick.touched = true
                        flick.zoomTo(flick.zoom * 2.5 > map.maxZoom + 0.01 && flick.zoom > 1.01
                                     ? 1 : flick.zoom * 2.5, Qt.point(mouse.x, mouse.y))
                    }
                }
            }
        }
    }

    // Required by the tile licence.
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
}
