import QtQuick 2.0
import Sailfish.Silica 1.0

// The room's pinned messages as their own view over the shared timeline model.
// The room page switches the core back to live when it is active again.
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
                    // The same rule the chat list follows: the relative timepoint carries no year,
                    // and a pin is exactly the kind of message that is years old.
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
                    // Back into the room, which then scrolls to the message. The room may not be
                    // the page underneath - reached from the info page too - so the stack is unwound.
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

        // The room can name pinned events the own server answers 404 for. Saying
        // "none" would contradict the banner, which counts the room's own list.
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
