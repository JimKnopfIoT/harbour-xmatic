import QtQuick 2.0
import Sailfish.Silica 1.0

// Pick the space a room moves to. Tapping a target adds the room there and
// removes it from the current space; the current space itself is hidden from
// the list, since moving a room onto itself means nothing.
Page {
    id: page

    property string currentSpaceId
    property string roomId
    property string roomName

    allowedOrientations: Orientation.All

    SilicaListView {
        id: spaceList

        anchors.fill: parent
        model: matrix.spaces

        header: PageHeader {
            title: qsTr("Move to space")
            description: page.roomName
        }

        delegate: RoomDelegate {
            // The current space is not a valid target; a zero-height invisible
            // row keeps the model indices aligned with the SDK's diffs.
            visible: model.id !== page.currentSpaceId
            contentHeight: visible ? Theme.itemSizeMedium : 0
            height: visible ? Theme.itemSizeMedium : 0

            onClicked: {
                matrix.moveRoomToSpace(page.currentSpaceId, model.id, page.roomId)
                pageStack.pop()
            }
        }

        ViewPlaceholder {
            enabled: spaceList.count <= 1
            text: qsTr("No other space")
            hintText: qsTr("Create another space first to move rooms between them.")
        }

        VerticalScrollDecorator { }
    }
}
