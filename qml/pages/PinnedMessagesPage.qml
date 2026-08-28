import QtQuick 2.0
import Sailfish.Silica 1.0

// The room's pinned messages, as their own view over the shared timeline
// model. Opening this page switches the core's timeline to the pinned focus;
// the room page switches it back to live when it becomes active again.
Page {
    id: page

    property string roomId: ""
    property string roomName: ""

    allowedOrientations: Orientation.All

    onStatusChanged: {
        if (status === PageStatus.Active) {
            matrix.openRoom(roomId, "pinned")
        }
    }

    SilicaListView {
        id: pinnedList

        anchors.fill: parent
        model: matrix.timeline

        header: PageHeader {
            title: qsTr("Pinned messages")
            description: page.roomName
        }

        delegate: ListItem {
            id: pinnedItem

            // Non-message rows (day dividers, virtual items) collapse instead
            // of showing as empty gaps.
            contentHeight: model.kind === "message"
                           ? pinnedColumn.height + 2 * Theme.paddingMedium : 0
            visible: model.kind === "message"

            onClicked: openMenu()

            Column {
                id: pinnedColumn

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
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: pinnedItem.highlighted ? Theme.secondaryHighlightColor
                                                  : Theme.secondaryColor
                    textFormat: Text.PlainText
                    // The same rule the chat list follows: the relative
                    // timepoint carries no year, and a pin is exactly the kind
                    // of message that is years old.
                    text: (model.senderName || model.sender || "")
                          + (model.timestamp > 0
                             ? " · " + Format.formatDate(
                                   new Date(model.timestamp),
                                   new Date(model.timestamp).getFullYear()
                                   === new Date().getFullYear()
                                   ? Formatter.TimepointRelative
                                   : Formatter.DateMedium)
                             : "")
                }

                Label {
                    width: parent.width
                    wrapMode: Text.Wrap
                    color: pinnedItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                    textFormat: Text.PlainText
                    text: model.body || ""
                }
            }

            menu: ContextMenu {
                MenuItem {
                    text: qsTr("Show in conversation")
                    // Back into the normal room window, which then scrolls to
                    // the message - no separate view, and the way back here
                    // stays the banner.
                    //
                    // The conversation is not necessarily the page underneath:
                    // this list is reached from the banner in the room *and*
                    // from the room's info page, and in the second case the
                    // page below is that info page. Asking it to jump did
                    // nothing at all, and what the user saw was the room at its
                    // end - which is where it opens when there is nothing
                    // unread. So the room is looked for in the stack and the
                    // stack is unwound back to it; a jump behind a page that is
                    // still on top would be lost the same way.
                    onClicked: {
                        var room = pageStack.find(function(candidate) {
                            return candidate.objectName === "roomPage"
                        })
                        if (room && room.jumpToPinned) {
                            room.jumpToPinned(model.eventId)
                            pageStack.pop(room)
                            return
                        }
                        pageStack.pop()
                    }
                }

                MenuItem {
                    text: qsTr("Unpin")
                    onClicked: matrix.pinMessage(model.eventId, false)
                }
            }
        }

        // The room can name pinned events that the own server does not have —
        // it answers each of them with 404. Saying "none" would contradict the
        // banner, which counts the room's list and is right to do so.
        ViewPlaceholder {
            enabled: pinnedList.count === 0
            text: matrix.pinnedEventIds.length > 0
                  ? qsTr("Pinned messages unavailable")
                  : qsTr("No pinned messages")
            hintText: matrix.pinnedEventIds.length > 0
                      ? qsTr("The server does not hand out these messages. They are older than this server's copy of the room.")
                      : qsTr("Long-press a message in the conversation to pin it.")
        }

        VerticalScrollDecorator { }
    }
}
