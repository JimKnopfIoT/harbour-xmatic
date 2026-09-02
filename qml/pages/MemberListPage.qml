import QtQuick 2.0
import Sailfish.Silica 1.0

// The members of one room, name over address. A row opens the profile page,
// where all actions live; the order comes from the core.
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

            // Loading the list can fail; silence would look empty.
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

            onClicked: pageStack.push(Qt.resolvedUrl("MemberProfilePage.qml"),
                                      { roomId: page.roomId, userId: model.userId })

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

        }

        ViewPlaceholder {
            enabled: memberList.count === 0
            text: qsTr("No members yet")
            hintText: qsTr("The people in this room show up here.")
        }

        VerticalScrollDecorator { }
    }
}
