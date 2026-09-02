import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// The room list. Ordering, filtering and unread counts all come from the sync
// service in the core; this view only renders what the model was told.
Page {
    id: page

    // Set when this is the start page: it then attaches the space overview so a
    // sideways swipe reaches it, instead of going through the menu.
    property bool isHome: false

    // Leaving the foreground aborts a running countdown at once: suppressing it
    // at expiry left it firing on return from a minimised app.
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

    // The remorse hangs on the page, not the row: a delegate destroyed by a
    // re-sort would execute it. And leaving the page counts as abort, not confirm.
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

        // No current item, ever: Qt Quick auto-selects row 0 and refocuses it on
        // every model reset, which took the keyboard from the search field per keystroke.
        currentIndex: -1

        anchors.fill: parent
        model: matrix.rooms

        // The list holds one page and grows only when asked, so reaching the end is
        // the moment. Asking again once everything is loaded costs nothing.
        onAtYEndChanged: {
            if (atYEnd && count > 0) {
                matrix.loadMoreRooms()
            }
        }

        header: Column {
            width: roomList.width

            PageHeader {
                title: qsTr("Rooms")
                // The core reconnects on its own; this only says so. A server that cannot do
                // this app's sync produces the same "offline", and waiting will not help.
                description: {
                    if (!matrix.serverSupported) {
                        return qsTr("This homeserver is not supported")
                    }
                    // Rows held against rooms the server counts: an ungrown sync window and one
                    // page asked for look identical. Not while searching - filtered count.
                    var counted = searchField.text.length > 0
                            ? ""
                            : qsTr("%1 of %2 rooms").arg(roomList.count)
                                    .arg(matrix.roomTotal >= 0
                                         ? matrix.roomTotal : "?")
                    if (matrix.syncState === "offline") {
                        var offline = qsTr("Offline — waiting for the network")
                        return counted.length > 0 ? offline + " · " + counted
                                                  : offline
                    }
                    return counted
                }

                // The device's security where the user actually is. Gone while everything is
                // green: an indicator that is always lit says nothing.
                MouseArea {
                    anchors.left: parent.left
                    anchors.leftMargin: Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    width: Theme.itemSizeExtraSmall
                    height: width
                    visible: SecurityStatus.needsAttention(matrix)
                    onClicked: pageStack.push(Qt.resolvedUrl("SecurityStatusPage.qml"))

                    SecurityLamp {
                        anchors.centerIn: parent
                        overall: true
                    }
                }
            }

            // Only when the server itself is the problem: the empty list needs
            // a reason, or it reads as a network hiccup that never clears.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.errorColor
                visible: !matrix.serverSupported
                text: qsTr("Your homeserver does not offer the sync this app needs (simplified sliding sync, MSC4186). Rooms cannot be loaded from it. A newer server version, or an account on a server that supports it, is required.")
            }

            SearchField {
                id: searchField

                width: parent.width
                placeholderText: qsTr("Search rooms")
                // The keyboard's word list is a store outside this sandbox and suggests what
                // it learned in other apps. Same hints as the password line.
                inputMethodHints: Qt.ImhNoPredictiveText
                                  | Qt.ImhSensitiveData
                                  | Qt.ImhNoAutoUppercase
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
                    // Without opening it: the point of the entry is not having
                    // to read the room at all.
                    text: qsTr("Mark as read")
                    visible: model.unread > 0 || model.mentions > 0
                    onClicked: matrix.markRoomRead(model.id)
                }

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
                    // On an invitation this declines it, so the entry says so. Read here, before
                    // anything asynchronous: the model may have moved on by then.
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

            // Above the room entries: "Make start page" hides when this list is the start
            // page, and anything below it would slide up and read as a way into a room.
            MenuItem {
                text: qsTr("Verify user")
                onClicked: pageStack.push(Qt.resolvedUrl("VerifyUserPage.qml"))
            }

            // The four ways into a room are one entry: a menu holds about five before the
            // last is out of reach, and these are all "start something".
            MenuItem {
                text: qsTr("Rooms")
                onClicked: pageStack.push(Qt.resolvedUrl("RoomActionsPage.qml"))
            }

            MenuItem {
                // Only shown when rooms is not already the start page; picking
                // it takes effect on the next start.
                text: qsTr("Make start page")
                visible: settings.startPage !== "rooms"
                onClicked: settings.startPage = "rooms"
            }

            MenuItem {
                text: qsTr("Account")
                onClicked: pageStack.push(Qt.resolvedUrl("AccountPage.qml"))
            }

        }

        VerticalScrollDecorator { }
    }
}
