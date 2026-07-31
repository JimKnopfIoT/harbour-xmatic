import QtQuick 2.0
import Sailfish.Silica 1.0

// Where something shared from another application lands: pick the room it goes
// to.
//
// Unlike forwarding, which returns to the conversation it came from, this ends
// in the room that was picked. The user arrives from outside the app and would
// otherwise be left looking at a list, with no sign of whether anything was
// sent.
Page {
    id: page

    /// Exactly one of these is set: a link or a piece of text arrives as a
    /// body, a file as a path plus its type.
    property string body: ""
    property string path: ""
    property string mimeType: ""

    allowedOrientations: Orientation.All

    SilicaListView {
        id: roomList

        anchors.fill: parent
        model: matrix.rooms

        header: PageHeader {
            title: qsTr("Send to")
            description: page.body.length > 0 ? page.body : page.path
        }

        delegate: ListItem {
            contentHeight: Theme.itemSizeSmall

            // A room one has only been invited to cannot receive anything yet.
            enabled: model.membership === "joined"
            opacity: enabled ? 1.0 : 0.4

            onClicked: {
                matrix.forwardToRoom(model.id, page.body, page.path, page.mimeType)
                pageStack.replace(Qt.resolvedUrl("RoomPage.qml"), {
                                      roomId: model.id,
                                      roomName: model.name,
                                      encrypted: model.encrypted
                                  })
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
                textFormat: Text.PlainText
                text: model.name
            }
        }

        ViewPlaceholder {
            enabled: roomList.count === 0
            text: qsTr("No rooms")
        }

        VerticalScrollDecorator { }
    }
}
