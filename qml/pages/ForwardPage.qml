import QtQuick 2.0
import Sailfish.Silica 1.0

// Pick a room to forward to. Forwarding re-sends rather than references: the
// target room encrypts under its own keys.
Page {
    id: page

    /// Exactly one of these is set.
    property string body: ""
    property string path: ""
    property string mimeType: ""

    allowedOrientations: Orientation.All

    SilicaListView {
        anchors.fill: parent
        model: matrix.rooms

        header: PageHeader {
            title: qsTr("Forward to")
        }

        delegate: ListItem {
            contentHeight: Theme.itemSizeSmall

            // Invitations cannot receive messages yet.
            enabled: model.membership === "joined"
            opacity: enabled ? 1.0 : 0.4

            onClicked: {
                matrix.forwardToRoom(model.id, page.body, page.path, page.mimeType)
                pageStack.pop()
            }

            Label {
                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                truncationMode: TruncationMode.Fade
                text: model.name
            }
        }

        VerticalScrollDecorator { }
    }
}
