import QtQuick 2.0
import Sailfish.Silica 1.0

// Rooms left or banned from but kept by the account. The chat list hides them,
// the server counts them; tapping one forgets it, behind a remorse.
Page {
    id: page

    property var rooms: []
    property string error: ""

    // Remorse guard as elsewhere: abort on minimise, never fire on an
    // inactive page.
    property var activeRemorse: null
    readonly property bool appForeground: Qt.application.active
    onAppForegroundChanged: {
        if (!appForeground && activeRemorse && activeRemorse.pending) {
            activeRemorse.cancel()
        }
    }

    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.leftRooms.load()

    Connections {
        target: matrix.leftRooms
        onLoaded: page.rooms = rooms
        onForgotten: {
            page.error = ""
            page.rooms = page.rooms.filter(function(room) { return room.id !== roomId })
        }
        onFailed: page.error = error
    }

    SilicaListView {
        id: listView

        anchors.fill: parent
        model: page.rooms

        header: Column {
            width: listView.width

            PageHeader {
                title: qsTr("Left rooms")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Rooms you left or declined in another app, or were removed or banned from. This device keeps them until they are forgotten, and the server counts a room you were removed or banned from until then. Tap one to forget it.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                textFormat: Text.PlainText
                visible: page.error.length > 0
                text: page.error
            }
        }

        delegate: ListItem {
            id: row

            contentHeight: Theme.itemSizeMedium

            onClicked: {
                var roomId = modelData.id
                page.activeRemorse = Remorse.popupAction(
                            page, qsTr("Forgetting room"), function() {
                                page.activeRemorse = null
                                if (page.status !== PageStatus.Active
                                        || !Qt.application.active) {
                                    return
                                }
                                matrix.leftRooms.forget(roomId)
                            })
            }

            Column {
                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    color: row.highlighted ? Theme.highlightColor : Theme.primaryColor
                    textFormat: Text.PlainText
                    text: modelData.name.length > 0 ? modelData.name : qsTr("Unnamed room")
                }

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryColor
                    //: State of a room the account is no longer in
                    text: modelData.banned ? qsTr("Banned") : qsTr("Left")
                }
            }
        }

        ViewPlaceholder {
            enabled: listView.count === 0
            text: qsTr("No left rooms")
        }

        VerticalScrollDecorator { }
    }
}
