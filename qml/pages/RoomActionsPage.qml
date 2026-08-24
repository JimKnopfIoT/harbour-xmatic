import QtQuick 2.0
import Sailfish.Silica 1.0

// The four ways into a room, on one page.
//
// They used to be four entries in the chat list's pull-down menu, which had
// grown to eight - past what a menu can hold on a phone, and every one of them
// a way of starting something rather than a thing one does daily. One entry
// leads here instead.
Page {
    id: page

    allowedOrientations: Orientation.All

    SilicaListView {
        anchors.fill: parent

        header: PageHeader {
            title: qsTr("Rooms")
        }

        model: ListModel {
            ListElement { action: "chat" }
            ListElement { action: "create" }
            ListElement { action: "join" }
            ListElement { action: "discover" }
        }

        delegate: ListItem {
            width: parent.width

            function label() {
                switch (model.action) {
                case "chat":     return qsTr("New chat")
                case "create":   return qsTr("New room")
                case "join":     return qsTr("Join room")
                default:         return qsTr("Discover rooms")
                }
            }

            onClicked: {
                // Replaced, not stacked: this page was the way to the dialog
                // and has nothing to say afterwards.
                switch (model.action) {
                case "chat":
                    pageStack.replace(Qt.resolvedUrl("NewChatDialog.qml"))
                    break
                case "create":
                    pageStack.replace(Qt.resolvedUrl("CreateRoomDialog.qml"))
                    break
                case "join":
                    pageStack.replace(Qt.resolvedUrl("JoinRoomDialog.qml"))
                    break
                default:
                    pageStack.replace(Qt.resolvedUrl("DirectoryPage.qml"))
                }
            }

            Label {
                anchors {
                    left: parent.left
                    right: parent.right
                    leftMargin: Theme.horizontalPageMargin
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                truncationMode: TruncationMode.Fade
                text: label()
            }
        }

        VerticalScrollDecorator {}
    }
}
