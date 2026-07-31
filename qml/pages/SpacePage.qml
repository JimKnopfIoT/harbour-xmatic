import QtQuick 2.0
import Sailfish.Silica 1.0

// The rooms of one space. A child that is itself a space opens as another
// SpacePage, so nested spaces just push further onto the stack.
//
// Only one space's rooms stream at a time in the core, so the stream is
// (re)opened whenever this page becomes active — which also covers popping
// back to it from a nested space or from a room. Closing is deliberately NOT
// tied to destruction: on a pop the revealed parent's activation must win, and
// destruction order is not guaranteed. The stream is instead closed by
// SpacesPage when the overview becomes active again.
//
// Below the joined rooms, the space's other linked children from the server's
// /hierarchy answer are offered as joinable.
Page {
    id: page

    property string spaceId
    property string spaceName

    allowedOrientations: Orientation.All

    onStatusChanged: {
        if (status === PageStatus.Active) {
            // Read now for a quick render, and once more shortly after: coming
            // back from "Add rooms" the change may still be syncing in.
            matrix.openSpace(spaceId)
            matrix.fetchSpaceHierarchy(spaceId)
            refreshTimer.restart()
        }
    }

    // A short delay lets an add or remove reach the server and come back before
    // the space's rooms are re-read, so the change actually shows.
    Timer {
        id: refreshTimer
        interval: 1200
        onTriggered: {
            matrix.openSpace(page.spaceId)
            matrix.fetchSpaceHierarchy(page.spaceId)
        }
    }

    // The linked-but-not-joined children of this space.
    ListModel {
        id: linkedModel
    }

    Connections {
        target: matrix
        onSpaceHierarchyReady: {
            if (spaceId !== page.spaceId) {
                return
            }
            linkedModel.clear()
            for (var i = 0; i < rooms.length; i++) {
                if (!rooms[i].joined) {
                    linkedModel.append(rooms[i])
                }
            }
        }
    }

    SilicaListView {
        id: roomList

        anchors.fill: parent
        model: matrix.spaceRooms

        header: PageHeader {
            title: page.spaceName.length > 0 ? page.spaceName : qsTr("Space")
        }

        PullDownMenu {
            MenuItem {
                text: qsTr("Add rooms")
                onClicked: pageStack.push(Qt.resolvedUrl("AddToSpacePage.qml"),
                                          { spaceId: page.spaceId })
            }

            MenuItem {
                text: qsTr("Refresh")
                onClicked: {
                    matrix.openSpace(page.spaceId)
                    matrix.fetchSpaceHierarchy(page.spaceId)
                }
            }
        }

        delegate: RoomDelegate {
            onClicked: {
                if (model.space) {
                    pageStack.push(Qt.resolvedUrl("SpacePage.qml"), {
                                       spaceId: model.id,
                                       spaceName: model.name
                                   })
                } else {
                    pageStack.push(Qt.resolvedUrl("RoomPage.qml"), {
                                       roomId: model.id,
                                       roomName: model.name,
                                       invited: model.membership === "invited",
                                       encrypted: model.encrypted
                                   })
                }
            }

            menu: ContextMenu {
                MenuItem {
                    text: qsTr("Move to space")
                    onClicked: pageStack.push(Qt.resolvedUrl("MoveToSpacePage.qml"), {
                                                  currentSpaceId: page.spaceId,
                                                  roomId: model.id,
                                                  roomName: model.name
                                              })
                }

                MenuItem {
                    text: qsTr("Remove from space")
                    onClicked: {
                        matrix.removeRoomFromSpace(page.spaceId, model.id)
                        refreshTimer.restart()
                    }
                }
            }
        }

        footer: Column {
            width: roomList.width
            visible: linkedModel.count > 0
            height: visible ? implicitHeight : 0

            SectionHeader {
                text: qsTr("Linked, not joined")
            }

            Repeater {
                model: linkedModel

                delegate: ListItem {
                    id: linkedItem

                    width: parent.width
                    contentHeight: linkedColumn.height + 2 * Theme.paddingMedium

                    onClicked: openMenu()

                    Column {
                        id: linkedColumn

                        anchors {
                            left: parent.left
                            leftMargin: Theme.horizontalPageMargin
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin
                            verticalCenter: parent.verticalCenter
                        }
                        spacing: Theme.paddingSmall

                        Label {
                            width: parent.width
                            truncationMode: TruncationMode.Fade
                            color: linkedItem.highlighted ? Theme.highlightColor
                                                          : Theme.primaryColor
                            textFormat: Text.PlainText
                            text: (model.name.length > 0 ? model.name : model.id)
                                  + (model.space ? " (" + qsTr("Space") + ")" : "")
                        }

                        Label {
                            width: parent.width
                            visible: text.length > 0
                            maximumLineCount: 2
                            wrapMode: Text.Wrap
                            elide: Text.ElideRight
                            font.pixelSize: Theme.fontSizeExtraSmall
                            color: linkedItem.highlighted ? Theme.secondaryHighlightColor
                                                          : Theme.secondaryColor
                            textFormat: Text.PlainText
                            text: model.topic
                        }

                        Label {
                            width: parent.width
                            font.pixelSize: Theme.fontSizeTiny
                            color: linkedItem.highlighted ? Theme.secondaryHighlightColor
                                                          : Theme.secondaryColor
                            textFormat: Text.PlainText
                            text: qsTr("%n member(s)", "", model.members)
                        }
                    }

                    menu: ContextMenu {
                        MenuItem {
                            text: qsTr("Join")
                            onClicked: {
                                var roomId = model.id
                                linkedItem.remorseAction(qsTr("Joining"), function() {
                                    matrix.joinRoomByAlias(roomId)
                                    refreshTimer.restart()
                                })
                            }
                        }
                    }
                }
            }
        }

        ViewPlaceholder {
            enabled: roomList.count === 0 && linkedModel.count === 0
            text: qsTr("No rooms in this space")
            hintText: qsTr("Add rooms with the pulldown menu, or rooms of this space that you have joined show up here.")
        }

        VerticalScrollDecorator { }
    }
}
