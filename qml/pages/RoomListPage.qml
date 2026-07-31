import QtQuick 2.0
import Sailfish.Silica 1.0

// The room list. Ordering, filtering and unread counts all come from the sync
// service in the core; this view only renders what the model was told.
Page {
    id: page

    // Set when this is the start page: it then attaches the space overview so a
    // sideways swipe reaches it, instead of going through the menu.
    property bool isHome: false

    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.startRoomList()

    onStatusChanged: {
        // Re-attach on every activation: opening a room replaces the attached
        // page, so it has to be restored when coming back.
        if (status === PageStatus.Active && isHome && !pageStack.nextPage(page)) {
            pageStack.pushAttached(Qt.resolvedUrl("SpacesPage.qml"), { isHome: false })
        }
    }

    SilicaListView {
        id: roomList

        anchors.fill: parent
        model: matrix.rooms

        header: Column {
            width: roomList.width

            PageHeader {
                title: qsTr("Rooms")
                // The core reconnects on its own; this only says that it is
                // doing so instead of leaving a silently stale list.
                description: matrix.syncState === "offline"
                             ? qsTr("Offline — waiting for the network") : ""
            }

            SearchField {
                id: searchField

                width: parent.width
                placeholderText: qsTr("Search rooms")
                onTextChanged: matrix.setRoomFilter(text)
            }

            // The core re-filters on every keystroke and the list comes back as
            // a full model reset, which drops keyboard focus from the field in
            // the header. Hand it straight back so typing is not interrupted
            // letter by letter.
            Connections {
                target: matrix.rooms
                onModelReset: {
                    if (searchField.text.length > 0 && !searchField.activeFocus) {
                        searchField.forceActiveFocus()
                    }
                }
            }
        }

        delegate: RoomDelegate {
            id: roomEntry

            onClicked: pageStack.push(Qt.resolvedUrl("RoomPage.qml"), {
                                          roomId: model.id,
                                          roomName: model.name,
                                          invited: model.membership === "invited",
                                          encrypted: model.encrypted
                                      })

            menu: ContextMenu {
                MenuItem {
                    text: model.favourite
                          ? qsTr("Remove from favourites") : qsTr("Favourite")
                    onClicked: matrix.setRoomFavourite(model.id, !model.favourite)
                }

                MenuItem {
                    // Low priority sinks the room to the bottom and silences its
                    // notifications.
                    text: model.lowPriority
                          ? qsTr("Normal priority") : qsTr("Low priority")
                    onClicked: matrix.setRoomLowPriority(model.id, !model.lowPriority)
                }

                MenuItem {
                    // The change syncs into the account's push rules, so it
                    // holds on every client, not only here.
                    text: model.muted ? qsTr("Unmute") : qsTr("Mute")
                    onClicked: matrix.setRoomMuted(model.id, !model.muted)
                }

                MenuItem {
                    // On an invitation this declines it, so the entry says so.
                    // The row is read before the remorse fires: by then the
                    // model may already have moved on.
                    text: model.membership === "invited"
                          ? qsTr("Decline invitation") : qsTr("Leave room")
                    onClicked: {
                        var leavingId = model.id
                        roomEntry.remorseAction(
                                    model.membership === "invited"
                                    ? qsTr("Declining") : qsTr("Leaving room"),
                                    function() { matrix.leaveRoom(leavingId) })
                    }
                }
            }
        }

        ViewPlaceholder {
            enabled: roomList.count === 0
            text: qsTr("No rooms")
            hintText: qsTr("Rooms you join show up here once the first sync is through.")
        }

        PullDownMenu {
            MenuItem {
                text: qsTr("About xmatic")
                onClicked: pageStack.push(Qt.resolvedUrl("AboutPage.qml"))
            }

            MenuItem {
                text: qsTr("New chat")
                onClicked: pageStack.push(Qt.resolvedUrl("NewChatDialog.qml"))
            }

            MenuItem {
                text: qsTr("New room")
                onClicked: pageStack.push(Qt.resolvedUrl("CreateRoomDialog.qml"))
            }

            MenuItem {
                text: qsTr("Join room")
                onClicked: pageStack.push(Qt.resolvedUrl("JoinRoomDialog.qml"))
            }

            MenuItem {
                text: qsTr("Discover rooms")
                onClicked: pageStack.push(Qt.resolvedUrl("DirectoryPage.qml"))
            }

            MenuItem {
                // Only shown when rooms is not already the start page; picking
                // it takes effect on the next start.
                text: qsTr("Make start page")
                visible: matrix.startPage !== "rooms"
                onClicked: matrix.startPage = "rooms"
            }

            MenuItem {
                text: qsTr("Account")
                onClicked: pageStack.push(Qt.resolvedUrl("AccountPage.qml"))
            }

            MenuItem {
                text: qsTr("Sign out")
                onClicked: pageStack.push(Qt.resolvedUrl("LogoutDialog.qml"))
            }
        }

        VerticalScrollDecorator { }
    }
}
