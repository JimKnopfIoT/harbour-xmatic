import QtQuick 2.0
import Sailfish.Silica 1.0

CoverBackground {
    id: cover

    // The app mark blown up far past the cover, so only a detail shows. Same
    // artwork as the launcher icon, which ties tile and icon together.
    Item {
        anchors.fill: parent
        clip: true

        Image {
            source: Qt.resolvedUrl("../images/xmatic-mark.png")
            // Framed so the visible rectangle stays inside the tile's silhouette. All
            // offsets are in cover widths, so the crop holds whatever aspect it has.
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
            // Only while there is no session: the cover sits on the home screen, and the
            // account's identifier is nobody's business there.
            visible: matrix.sessionState !== "signed-in"
            text: matrix.busy ? qsTr("signing in…") : qsTr("not signed in")
        }

        // That nothing is arriving - reported from the field. This app has no
        // background service, so the cover is how it is watched; same words as the pages.
        Label {
            width: parent.width
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryColor
            wrapMode: Text.Wrap
            visible: matrix.sessionState === "signed-in"
                     && matrix.syncState === "offline"
            text: qsTr("Offline — waiting for the network")
        }

        Label {
            // Rooms with news over the messages they hold, the compact notation the
            // daemonless messengers use, free of any account detail.
            width: parent.width
            font.pixelSize: Theme.fontSizeHuge
            color: Theme.highlightColor
            visible: matrix.sessionState === "signed-in" && matrix.unreadRooms > 0
            // The same "+" the row badge carries: the total is a sum of counts that stop
            // at the sync window, so where one stopped the sum is a floor.
            text: matrix.unreadRooms + "/" + matrix.unreadMessages
                  + (matrix.unreadCapped ? "+" : "")
        }
    }
}
