import QtQuick 2.0
import QtGraphicalEffects 1.0
import Sailfish.Silica 1.0

// One profile picture. Initials first so no row jumps, decoded at display size
// because people upload full camera resolution, keyed by address so one download serves all.
Item {
    id: avatar

    /// The MXC address of the picture, or empty for none.
    property string source: ""
    /// Whose picture it is — the initials fall back to this.
    property string name: ""
    property int size: Theme.iconSizeMedium

    width: size
    height: size

    /// Only through the bridge: a refused file (SVG, oversized) answers empty.
    property string _url: ""

    function _refresh() {
        _url = ""
        if (source.length === 0) {
            return
        }
        if (matrix.mediaPath(source).length > 0) {
            _url = matrix.imageSource(source)
        } else {
            matrix.requestAvatar(source)
        }
    }

    onSourceChanged: _refresh()
    Component.onCompleted: _refresh()

    Connections {
        target: matrix
        onMediaReady: {
            if (key === avatar.source) {
                avatar._url = matrix.imageSource(key)
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        visible: avatar._url.length === 0
        color: Theme.rgba(Theme.highlightBackgroundColor, Theme.opacityFaint)

        Label {
            anchors.centerIn: parent
            font.pixelSize: avatar.size / 2.5
            color: Theme.highlightColor
            textFormat: Text.PlainText
            text: {
                // The first letter of the name, or of the address without its
                // sigil — a bare "@" or "#" would say nothing at all.
                var n = avatar.name.replace(/^[@#!]/, "").trim()
                return n.length > 0 ? n.charAt(0).toUpperCase() : "?"
            }
        }
    }

    // Genuinely round: `clip` clips to a rectangle, never to a radius, and the
    // frame-over-picture form left a ring drawn across every square picture.
    Image {
        id: picture

        anchors.fill: parent
        visible: avatar._url.length > 0
        asynchronous: true
        fillMode: Image.PreserveAspectCrop
        sourceSize.width: avatar.size
        sourceSize.height: avatar.size
        source: avatar._url

        layer.enabled: visible
        layer.effect: OpacityMask {
            maskSource: Rectangle {
                width: picture.width
                height: picture.height
                radius: width / 2
            }
        }
    }
}
