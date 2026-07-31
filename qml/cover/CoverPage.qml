import QtQuick 2.0
import Sailfish.Silica 1.0

CoverBackground {
    id: cover

    // The app mark again, but blown up far past the cover so only a detail of
    // it shows: the m and the glyphs falling around it. Same artwork as the
    // launcher icon, which is what ties tile and icon together.
    Item {
        anchors.fill: parent
        clip: true

        Image {
            source: Qt.resolvedUrl("../images/xmatic-mark.png")
            // Framed so the whole visible rectangle stays inside the tile's
            // silhouette — the mark no longer fills its square, so a wider crop
            // would drag the rounded edge across the cover as a stray arc. All
            // offsets are in cover widths, so the crop holds whatever aspect
            // the cover has.
            width: cover.width * 2.30
            height: width
            x: -cover.width * 0.60
            y: -cover.width * 0.45
            opacity: 0.32
            smooth: true
            asynchronous: true
        }
    }

    Column {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.topMargin: Theme.paddingLarge
        anchors.leftMargin: Theme.paddingMedium
        anchors.rightMargin: Theme.paddingMedium
        spacing: Theme.paddingSmall

        Label {
            width: parent.width
            font.pixelSize: Theme.fontSizeLarge
            text: "xmatic"
        }

        Label {
            width: parent.width
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryColor
            wrapMode: Text.Wrap
            text: matrix.sessionState === "signed-in"
                  ? matrix.userId
                  : (matrix.busy ? qsTr("signing in…") : qsTr("not signed in"))
        }
    }
}
