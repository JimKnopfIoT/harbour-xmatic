import QtQuick 2.0
import Sailfish.Silica 1.0

// Everything that can be done with one message, on a page. For landscape,
// where the conversation keeps four menu rows and a message's menu needs six.
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

    /// What `saveAttachment` needs: id, msgtype, media. Copied out of the delegate,
    /// whose model row stops existing once it leaves the cache.
    property var item

    allowedOrientations: Orientation.All

    // What focuses the composer or pushes a page is handed back to the room page:
    // a `push()` during this page's own `pop()` returns nothing to connect to.
    function handBack(kind) {
        roomPage.pendingAction = {
            "kind": kind,
            "eventId": page.eventId,
            "senderName": page.senderName,
            "body": page.body,
            // For forwarding an attachment, which needs the row's own data.
            "item": page.item,
            // Deleting hands back for another reason: its countdown has to run where it
            // can be seen and called off, and this page is gone a moment later.
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

            // One condition per entry, never on the column: hiding the container takes
            // every other action with it.

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
                onClicked: page.handBack("thread")
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


            // Text and attachments both. Hidden for a picture and shown for other files,
            // this forwarded "holiday.pdf" as a sentence - reported as "there is no Forward".
            ListItem {
                contentHeight: Theme.itemSizeSmall
                visible: page.canSave || page.body.length > 0
                onClicked: {
                    if (page.canSave) {
                        // Handed back rather than pushed: the file may still have to be fetched, and
                        // the room page opens the picker when it lands.
                        page.handBack("forwardAttachment")
                        return
                    }
                    // Replaces rather than pushes: forwarding pops one page when it is done, and
                    // that has to land in the conversation.
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
                visible: page.eventId.length > 0
                onClicked: page.handBack("react")
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
        }

        VerticalScrollDecorator { }
    }
}
