import QtQuick 2.0
import Sailfish.Silica 1.0

// Everything about one room on a single page: what it is, who is in it, and
// the actions that belong to the room rather than to the running conversation.
//
// This page exists for two reasons. The room's pull-down had grown to ten
// entries, which is barely draggable on a small landscape screen — everything
// that is not about the conversation itself lives here now. And the two lines
// that tell a user their room is old, the internal id and the room version,
// had no place at all: the room-upgrade case was reported by someone who
// worked it out from exactly those two lines in another client.
//
// Everything arrives in one `room.info` reply, read from the room's local
// state, so the page is filled the moment it opens. The states that can be
// changed here are held in properties of their own rather than read out of the
// reply: a QML property-change signal cannot be raised by hand, so a switch
// bound into the reply object would not follow its own tap.
Page {
    id: page

    property string roomId
    property string roomName
    // An invitation not yet accepted: the room can be looked at — its topic is
    // what tells someone whether to accept — but nothing inside it can be done.
    property bool invited: false

    // The core's answer. Empty until it arrives, which is the state every
    // binding below is written for.
    property var info: ({})
    property bool infoLoaded: false

    // Held separately because they are written from this page.
    property bool muted: false
    property bool favourite: false
    property bool lowPriority: false
    property bool encrypted: false

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

    Component.onCompleted: matrix.loadRoomInfo(roomId)

    // The name shown to the user, which is what the confirmation has to say
    // back to them — the push site may not have known it.
    readonly property string displayName: page.roomName.length > 0
                                          ? page.roomName
                                          : (page.info.name || qsTr("this room"))

    // Two steps, because leaving cannot be undone: a dialog that names the
    // room, then the remorse as the undo.
    function confirmLeave() {
        var dialog = pageStack.push(
                    Qt.resolvedUrl("ConfirmDialog.qml"),
                    {
                        question: qsTr("Really leave this room?"),
                        subject: page.displayName,
                        explanation: qsTr("The room is left and forgotten. It disappears from the chat list, and getting back in needs a new invitation or a public address."),
                        acceptLabel: qsTr("Leave")
                    })
        dialog.accepted.connect(function() { page.startLeave() })
    }

    function startLeave() {
        page.activeRemorse = Remorse.popupAction(page, qsTr("Leaving room"), function() {
            // A remorse whose page went away was executed by Silica, not by
            // the user (RemorsePopup.qml deliberately fires on
            // PageStatus.Deactivating). Going back means abort.
            if (page.status !== PageStatus.Active || !Qt.application.active) {
                return
            }
            matrix.leaveRoom(page.roomId)
            // Two levels: the conversation of a room just left must not stay
            // behind this page.
            var roomPage = pageStack.previousPage(page)
            var below = roomPage ? pageStack.previousPage(roomPage) : null
            if (below) {
                pageStack.pop(below)
            } else {
                pageStack.pop()
            }
        })
    }

    Connections {
        target: matrix
        onRoomInfoReady: {
            if (info.roomId !== page.roomId) {
                return
            }
            page.info = info
            page.muted = info.muted === true
            page.favourite = info.favourite === true
            page.lowPriority = info.lowPriority === true
            page.encrypted = info.encrypted === true
            page.infoLoaded = true
        }
    }

    SilicaFlickable {
        id: flickable

        anchors.fill: parent
        contentHeight: content.height + Theme.paddingLarge

        VerticalScrollDecorator {}

        PullDownMenu {
            // Deliberately the only entry: leaving is the one action here that
            // must not sit between the taps that merely inspect the room.
            MenuItem {
                text: qsTr("Leave room")
                onClicked: page.confirmLeave()
            }
        }

        Column {
            id: content

            width: page.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: page.roomName.length > 0 ? page.roomName
                                                : (page.info.name || qsTr("Room"))
                description: page.info.alias || ""
            }

            Avatar {
                anchors.horizontalCenter: parent.horizontalCenter
                size: Theme.itemSizeLarge
                source: page.info.avatar || ""
                name: page.roomName.length > 0 ? page.roomName : (page.info.name || "")
            }

            // The one thing a room says about itself — and until now xmatic
            // showed it only for rooms one had not joined yet.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: text.length > 0
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                color: Theme.secondaryHighlightColor
                font.pixelSize: Theme.fontSizeSmall
                text: page.info.topic || ""
            }

            // An upgraded room leads on: the same action the banner in the
            // conversation carries, in the place where a user goes looking for
            // what is wrong with the room.
            BackgroundItem {
                width: parent.width
                height: visible ? Theme.itemSizeSmall : 0
                visible: page.info.successor !== undefined && page.info.successor !== null
                onClicked: matrix.followSuccessor(page.roomId)

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.NoWrap
                    truncationMode: TruncationMode.Fade
                    color: Theme.highlightColor
                    text: (page.info.successor && page.info.successor.joined)
                          ? qsTr("Replaced — open the new room")
                          : qsTr("Replaced — join the new room")
                }
            }

            // The way back into the room this one grew out of. Only offered
            // while that room is still joined; otherwise there is nothing
            // behind the entry.
            BackgroundItem {
                width: parent.width
                height: visible ? Theme.itemSizeSmall : 0
                visible: page.info.predecessor !== undefined
                         && page.info.predecessor !== null
                         && page.info.predecessor.joined === true
                onClicked: pageStack.push(Qt.resolvedUrl("RoomPage.qml"),
                                          { roomId: page.info.predecessor.roomId,
                                            roomName: "" })

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.NoWrap
                    truncationMode: TruncationMode.Fade
                    color: Theme.highlightColor
                    text: qsTr("Older messages in the previous room")
                }
            }

            SectionHeader {
                visible: !page.invited
                text: qsTr("People and messages")
            }

            BackgroundItem {
                width: parent.width
                height: visible ? Theme.itemSizeSmall : 0
                visible: !page.invited
                onClicked: pageStack.push(Qt.resolvedUrl("MemberListPage.qml"), {
                                              roomId: page.roomId,
                                              roomName: page.roomName
                                          })

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.NoWrap
                    truncationMode: TruncationMode.Fade
                    // The invited are counted apart: they are not in the room
                    // yet, and a single number would not match the member list.
                    text: page.info.invitedMembers > 0
                          ? qsTr("Members: %1 (%2 invited)")
                            .arg(page.info.joinedMembers || 0)
                            .arg(page.info.invitedMembers)
                          : qsTr("Members: %1").arg(page.info.joinedMembers || 0)
                }
            }

            BackgroundItem {
                width: parent.width
                height: visible ? Theme.itemSizeSmall : 0
                visible: !page.invited
                onClicked: pageStack.push(Qt.resolvedUrl("PinnedMessagesPage.qml"), {
                                              roomId: page.roomId,
                                              roomName: page.roomName
                                          })

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.NoWrap
                    truncationMode: TruncationMode.Fade
                    text: qsTr("Pinned messages")
                }
            }

            BackgroundItem {
                width: parent.width
                height: visible ? Theme.itemSizeSmall : 0
                visible: !page.invited
                onClicked: pageStack.push(Qt.resolvedUrl("InviteToRoomDialog.qml"), {
                                              roomId: page.roomId,
                                              roomName: page.roomName
                                          })

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.NoWrap
                    truncationMode: TruncationMode.Fade
                    text: qsTr("Invite")
                }
            }

            SectionHeader {
                visible: !page.invited
                text: qsTr("This room for me")
            }

            // Also in the chat list's context menu, where they are a direct
            // action on the row. Here they additionally show their state — the
            // list can only show that something is set, not offer the reverse
            // reading at a glance.
            TextSwitch {
                visible: !page.invited
                text: qsTr("Mute notifications")
                checked: page.muted
                automaticCheck: false
                enabled: page.infoLoaded
                onClicked: {
                    page.muted = !page.muted
                    matrix.setRoomMuted(page.roomId, page.muted)
                }
            }

            TextSwitch {
                visible: !page.invited
                text: qsTr("Favourite")
                checked: page.favourite
                automaticCheck: false
                enabled: page.infoLoaded
                onClicked: {
                    page.favourite = !page.favourite
                    matrix.setRoomFavourite(page.roomId, page.favourite)
                    // The core clears the other tag; the switch has to follow,
                    // or the page claims a state the room does not have.
                    if (page.favourite) {
                        page.lowPriority = false
                    }
                }
            }

            TextSwitch {
                visible: !page.invited
                text: qsTr("Low priority")
                description: qsTr("Sorts to the bottom of the list and stays quiet")
                checked: page.lowPriority
                automaticCheck: false
                enabled: page.infoLoaded
                onClicked: {
                    page.lowPriority = !page.lowPriority
                    matrix.setRoomLowPriority(page.roomId, page.lowPriority)
                    if (page.lowPriority) {
                        page.favourite = false
                    }
                }
            }

            SectionHeader {
                text: qsTr("Details")
            }

            DetailItem {
                label: qsTr("Encryption")
                value: page.encrypted ? qsTr("End-to-end encrypted")
                                      : qsTr("Not encrypted")
            }

            // One-way, so it asks first — the same remorse timer the room's
            // pull-down used to carry.
            BackgroundItem {
                width: parent.width
                height: visible ? Theme.itemSizeSmall : 0
                visible: page.infoLoaded && !page.encrypted
                onClicked: page.activeRemorse = Remorse.popupAction(page, qsTr("Turning on encryption"), function() {
                    // Leaving the page aborts; Silica's own behaviour is to
                    // execute on PageStatus.Deactivating, which for a one-way
                    // switch is the wrong direction.
                    if (page.status !== PageStatus.Active || !Qt.application.active) {
                        return
                    }
                    matrix.enableEncryption(page.roomId)
                    page.encrypted = true
                })

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.NoWrap
                    truncationMode: TruncationMode.Fade
                    color: Theme.highlightColor
                    text: qsTr("Turn on encryption")
                }
            }

            DetailItem {
                label: qsTr("Address")
                visible: value.length > 0
                value: page.info.alias || ""
            }

            DetailItem {
                label: qsTr("Access")
                visible: page.info.isPublic !== undefined && page.info.isPublic !== null
                value: page.info.isPublic ? qsTr("Public") : qsTr("On invitation")
            }

            // The two lines that say "this room is old".
            DetailItem {
                label: qsTr("Room version")
                visible: value.length > 0
                value: page.info.version || ""
            }

            // The id is offered for copying because it is the only handle left
            // on a room whose address has moved on to its successor.
            BackgroundItem {
                width: parent.width
                height: roomIdDetail.height + Theme.paddingSmall

                onClicked: {
                    Clipboard.text = page.info.roomId || page.roomId
                    copiedHint.visible = true
                }

                DetailItem {
                    id: roomIdDetail

                    width: parent.width
                    anchors.verticalCenter: parent.verticalCenter
                    label: qsTr("Room ID")
                    value: page.info.roomId || page.roomId
                }
            }

            Label {
                id: copiedHint

                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: false
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Room ID copied")
            }
        }
    }
}
