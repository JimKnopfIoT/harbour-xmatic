import QtQuick 2.0
import Sailfish.Silica 1.0

// The members of one room. The primary line is the display name, the line
// below it the Matrix address — the address is the point of the page, so it
// is always shown. Tapping a member opens the actions: message, verify, copy
// the address, and — power levels permitting — remove from the room. Ordering
// (most powerful first, then by name) and the permission flags come from the
// core; the server enforces the levels again on removal.
Page {
    id: page

    property string roomId
    property string roomName

    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.loadMembers(roomId)

    SilicaListView {
        id: memberList

        anchors.fill: parent
        model: matrix.members

        header: Column {
            width: memberList.width

            PageHeader {
                title: qsTr("Members")
                description: page.roomName
            }

            // Verifying or removing can fail (no encryption identity yet, the
            // server refuses); without this line the action fails silently.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                visible: matrix.lastError.length > 0
                text: matrix.lastError
            }
        }

        delegate: ListItem {
            id: memberItem

            contentHeight: memberColumn.height + 2 * Theme.paddingMedium

            onClicked: openMenu()

            Avatar {
                id: memberAvatar

                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                size: Theme.iconSizeMedium
                source: model.avatar || ""
                name: model.displayName
            }

            Column {
                id: memberColumn

                anchors {
                    left: memberAvatar.right
                    leftMargin: Theme.paddingMedium
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                spacing: Theme.paddingSmall

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    color: memberItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                    textFormat: Text.PlainText
                    text: {
                        var suffix = ""
                        if (model.membership === "invite") {
                            suffix = " · " + qsTr("invited")
                        } else if (model.power >= 100) {
                            suffix = " · " + qsTr("Admin")
                        } else if (model.power >= 50) {
                            suffix = " · " + qsTr("Moderator")
                        }
                        return model.displayName + suffix
                    }
                }

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: memberItem.highlighted ? Theme.secondaryHighlightColor
                                                  : Theme.secondaryColor
                    textFormat: Text.PlainText
                    text: model.userId
                }
            }

            menu: ContextMenu {
                MenuItem {
                    text: qsTr("Send direct message")
                    visible: !model.isSelf
                    onClicked: matrix.startDirectChat(model.userId)
                }

                MenuItem {
                    text: qsTr("Verify")
                    visible: !model.isSelf
                    onClicked: {
                        matrix.requestVerification(model.userId)
                        pageStack.push(Qt.resolvedUrl("VerificationPage.qml"))
                    }
                }

                MenuItem {
                    text: qsTr("Copy address")
                    onClicked: Clipboard.text = model.userId
                }

                MenuItem {
                    text: qsTr("Remove from room")
                    visible: model.canRemove
                    onClicked: memberItem.remorseAction(qsTr("Removing"), function() {
                        matrix.removeMember(page.roomId, model.userId)
                    })
                }
            }
        }

        ViewPlaceholder {
            enabled: memberList.count === 0
            text: qsTr("No members yet")
            hintText: qsTr("The people in this room show up here.")
        }

        VerticalScrollDecorator { }
    }
}
