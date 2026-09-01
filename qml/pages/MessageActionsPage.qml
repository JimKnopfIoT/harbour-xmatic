import QtQuick 2.0
import Sailfish.Silica 1.0

// Everything that can be done with one message, on a page of its own.
//
// This exists because of the landscape case: the screen is 1080 px high there,
// and after the room's name strip, the pinned banner and the composer the
// conversation keeps under 600 — four menu rows, where a message's own menu
// needs six. A context menu cannot scroll (Silica's is a plain column without
// a flickable, and flicking closes it), so the entries that do not fit move
// here, where a page has the whole screen.
//
// In portrait the menu still holds every entry and this page is not used.
Page {
    id: page

    /// The room this message lives in. Reply and edit are carried out there —
    /// they put the cursor in its composer.
    property var roomPage

    property string eventId
    /// A message whose send failed has no event id; this names it instead.
    property string txnId
    property bool unsent: false
    property string body
    property string senderName
    property bool isOwn: false
    property bool editable: false
    property bool isImage: false
    property bool canSave: false

    /// What saveAttachment needs: id, msgtype and the media map. Copied out of
    /// the delegate rather than passed along as the model row, which stops
    /// existing the moment the row scrolls out of the cache.
    property var item

    allowedOrientations: Orientation.All

    // Reply and edit hand the work back to the room page instead of doing it
    // here: both put focus into the composer, and focus set while the page
    // stack is still animating does not stick. The room page applies it when
    // it is on top again.
    function handBack(kind) {
        roomPage.pendingAction = {
            "kind": kind,
            "eventId": page.eventId,
            "senderName": page.senderName,
            "body": page.body,
            // Deleting hands back for a different reason than reply and edit:
            // its countdown has to run where it can be seen and called off,
            // and this page is gone a moment later.
            "txnId": page.txnId,
            "unsent": page.unsent
        }
        pageStack.pop()
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        Column {
            id: content

            width: parent.width

            PageHeader {
                title: qsTr("Message")
            }

            // One condition per entry, never on the column: hiding the
            // container would take every other action with it and leave the
            // page without a single thing to do.
            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.body.length > 0
                onClicked: {
                    Clipboard.text = page.body
                    pageStack.pop()
                }
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("Copy")
                }
            }

            ListItem {
                contentHeight: Theme.itemSizeSmall
                onClicked: page.handBack("reply")
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("Reply")
                }
            }

            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.canSave
                onClicked: {
                    roomPage.saveAttachment(page.item)
                    pageStack.pop()
                }
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("Save")
                }
            }

            // Text and attachments both, and each as itself.
            //
            // This entry used to be hidden for a picture and shown for every
            // other attachment - where `body` is the file name, so it forwarded
            // "holiday.pdf" as a sentence. A picture could only be forwarded
            // from the full-screen view, which is not where anybody looks for
            // it. Reported as "there is no Forward".
            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.canSave || page.body.length > 0
                onClicked: {
                    if (page.canSave) {
                        // Pushed, not replaced: the file may still have to be
                        // fetched, and the room page opens the picker when it
                        // lands. This list goes with the pop below it.
                        var item = page.item
                        pageStack.pop()
                        page.roomPage.forwardAttachment(item)
                        return
                    }
                    // Replaces rather than pushes: forwarding pops one page
                    // when it is done, and that has to land back in the
                    // conversation, not on this list.
                    pageStack.replace(Qt.resolvedUrl("ForwardPage.qml"), {
                                          body: page.body
                                      })
                }
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("Forward")
                }
            }

            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.isOwn && page.editable
                onClicked: page.handBack("edit")
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("Edit")
                }
            }

            ListItem {
                contentHeight: Theme.itemSizeSmall
                // As in the room's own menu: hidden only where the room has
                // answered that this account may not pin.
                visible: page.eventId.length > 0
                         && matrix.roomPermissions.pin !== false
                onClicked: {
                    matrix.pinMessage(page.eventId, true)
                    pageStack.pop()
                }
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("Pin")
                }
            }

            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.eventId.length > 0
                onClicked: {
                    var target = page.eventId
                    pageStack.pop()
                    page.roomPage.openThread(target)
                }
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("Reply in thread")
                }
            }

            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.eventId.length > 0
                onClicked: {
                    var target = page.eventId
                    pageStack.pop()
                    page.roomPage.pickReaction(target)
                }
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("React")
                }
            }

            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.unsent
                onClicked: {
                    matrix.retryMessage(page.txnId)
                    pageStack.pop()
                }
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: qsTr("Send again")
                }
            }

            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.isOwn
                onClicked: page.handBack("delete")
                Label {
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    text: page.unsent ? qsTr("Discard") : qsTr("Delete")
                    color: Theme.errorColor
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
