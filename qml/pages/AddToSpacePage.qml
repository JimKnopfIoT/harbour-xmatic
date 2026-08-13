import QtQuick 2.0
import Sailfish.Silica 1.0

// Pick rooms to add to a space. The list is the ordinary chat list (spaces are
// already excluded from it in the core). Long-pressing a room adds it to the
// space right away — no selecting and confirming.
Page {
    id: page

    property string spaceId

    allowedOrientations: Orientation.All

    SilicaListView {
        id: roomList

        anchors.fill: parent
        model: matrix.rooms

        header: Column {
            width: roomList.width

            PageHeader {
                title: qsTr("Add rooms")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Press and hold a room to add it to the space.")
            }
        }

        delegate: ListItem {
            id: roomItem

            contentHeight: Theme.itemSizeMedium

            // The whole point is the long-press action, so a tap just opens the
            // context menu instead of doing nothing.
            onClicked: openMenu()

            menu: ContextMenu {
                MenuItem {
                    text: qsTr("Add to space")
                    onClicked: {
                        var roomId = model.id
                        roomItem.remorseAction(qsTr("Adding to space"), function() {
                            // Leaving the page aborts. Silica would otherwise
                            // execute the action on PageStatus.Deactivating.
                            if (page.status !== PageStatus.Active) {
                                return
                            }
                            matrix.addRoomToSpace(page.spaceId, roomId)
                        })
                    }
                }
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
                color: roomItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                text: model.name
            }
        }

        ViewPlaceholder {
            enabled: roomList.count === 0
            text: qsTr("No rooms")
            hintText: qsTr("Join or start a chat first, then add it to a space.")
        }

        VerticalScrollDecorator { }
    }
}
