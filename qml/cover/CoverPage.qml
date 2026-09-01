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
            // Only while there is no session: the cover sits on the home
            // screen, and the account's identifier is nobody's business
            // there. Signed in, the tile shows the mark and nothing else.
            visible: matrix.sessionState !== "signed-in"
            text: matrix.busy ? qsTr("signing in…") : qsTr("not signed in")
        }

        // What the cover could not say until now: that nothing is arriving.
        //
        // Reported from the field - "if no net and waiting for connection,
        // xmatic shows it only under the room's name, but not in cover; need
        // in cover, first of all". Rightly first of all: the room list and the
        // room say it to somebody who is already looking at the app, and this
        // app has no background service, so the cover *is* how it is watched.
        // A tile that shows a count and nothing else claims that count is
        // current, and while the sync is down it is not.
        //
        // The same state and, word for word, the same sentence as the two
        // pages: one connection must not be described in two ways by an app
        // whose tile and whose list are read minutes apart by one person.
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
            // Rooms with news over the messages they hold, 2/7 say — the
            // compact notation the daemonless messengers on this platform
            // use, and deliberately free of any account detail: the cover
            // sits on the home screen.
            width: parent.width
            font.pixelSize: Theme.fontSizeHuge
            color: Theme.highlightColor
            visible: matrix.sessionState === "signed-in" && matrix.unreadRooms > 0
            // The same "+" the row badge carries, for the same reason: the
            // total is a sum of counts that stop at the sync window, so where
            // one of them stopped there the sum is a floor.
            text: matrix.unreadRooms + "/" + matrix.unreadMessages
                  + (matrix.unreadCapped ? "+" : "")
        }
    }
}
