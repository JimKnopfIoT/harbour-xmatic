import QtQuick 2.0
import Sailfish.Silica 1.0

// The room list. Ordering, filtering and unread counts all come from the sync
// service in the core; this view only renders what the model was told.
Page {
    id: page

    // Set when this is the start page: it then attaches the space overview so a
    // sideways swipe reaches it, instead of going through the menu.
    property bool isHome: false

    // Leaving the foreground aborts a running countdown at once, instead of only
    // suppressing it when it expires. Suppressing at expiry was not enough:
    // minimising the app and coming back inside the four seconds left the
    // countdown running, and it fired on return. The remorse object has to be
    // kept for that - Remorse.popupAction() hands it back.
    property var activeRemorse: null
    readonly property bool appForeground: Qt.application.active
    onAppForegroundChanged: {
        if (!appForeground && activeRemorse && activeRemorse.pending) {
            activeRemorse.cancel()
        }
    }


    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.startRoomList()

    onStatusChanged: {
        // Re-attach on every activation: opening a room replaces the attached
        // page, so it has to be restored when coming back.
        if (status === PageStatus.Active && isHome && !pageStack.nextPage(page)) {
            pageStack.pushAttached(Qt.resolvedUrl("SpacesPage.qml"), { isHome: false })
        }
    }

    // Leaving asks first and says which room, then runs the remorse as the
    // undo. Two details are deliberate.
    //
    // The remorse hangs on the page, not on the row: a RemorseItem inside a
    // delegate executes its action when that delegate is destroyed, and this
    // list re-sorts on every incoming message — an unrelated message in an
    // unrelated room could therefore have completed the countdown.
    //
    // And the callback checks that the page is still the active one. Silica
    // fires a running remorse on PageStatus.Deactivating by design ("if the
    // page is changed then execute immediately"), so swiping over to the
    // spaces or opening another room used to count as confirmation. Here it
    // counts as the abort the user meant.
    function confirmLeave(roomId, roomName, invited) {
        var dialog = pageStack.push(
                    Qt.resolvedUrl("ConfirmDialog.qml"),
                    {
                        question: invited ? qsTr("Really decline this invitation?")
                                          : qsTr("Really leave this room?"),
                        subject: roomName,
                        explanation: invited
                                     ? qsTr("The invitation is gone afterwards. You can only get back in if somebody invites you again.")
                                     : qsTr("The room is left and forgotten. It disappears from the chat list, and getting back in needs a new invitation or a public address."),
                        acceptLabel: invited ? qsTr("Decline") : qsTr("Leave")
                    })
        dialog.accepted.connect(function() {
            page.activeRemorse = Remorse.popupAction(page,
                                invited ? qsTr("Declining") : qsTr("Leaving room"),
                                function() {
                                    if (page.status !== PageStatus.Active || !Qt.application.active) {
                                        return
                                    }
                                    matrix.leaveRoom(roomId)
                                })
        })
    }

    SilicaListView {
        id: roomList

        // No current item, ever. Qt Quick auto-selects row 0 as soon as the
        // model has rows unless the index was cleared explicitly, and on every
        // model reset it recreates that current item and gives it focus inside
        // the view's focus scope (QQuickItemViewPrivate::updateCurrent →
        // setFocus(true)). The search field lives in this view's header, so
        // each keystroke — the core answers a filter with a full reset — took
        // the keyboard away from it. Measured on the device: with the index
        // left at Qt's default the focus fell after every reset; with -1 set
        // here it never did. (The 0.9.1 hand-back on `modelReset` ran before
        // the theft and therefore never fired.)
        currentIndex: -1

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
                    // The row is read here, before anything asynchronous
                    // starts: by then the model may already have moved on.
                    text: model.membership === "invited"
                          ? qsTr("Decline invitation") : qsTr("Leave room")
                    onClicked: page.confirmLeave(model.id, model.name,
                                                 model.membership === "invited")
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
