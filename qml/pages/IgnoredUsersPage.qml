import QtQuick 2.0
import Sailfish.Silica 1.0

// The account's ignored users (`m.ignored_user_list`). The list lives on the
// homeserver, not on this device, so it holds in every client. Tapping a row
// stops ignoring, behind a remorse.
Page {
    id: page

    property var users: []

    // Remorse guard as elsewhere: abort on minimise, never fire on an
    // inactive page.
    property var activeRemorse: null
    readonly property bool appForeground: Qt.application.active
    onAppForegroundChanged: {
        if (!appForeground && activeRemorse && activeRemorse.pending) {
            activeRemorse.cancel()
        }
    }

    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.loadIgnoredUsers()

    Connections {
        target: matrix
        onIgnoredUsersReady: page.users = users
        // An unignore elsewhere (a profile page) changes this list too.
        onMemberChanged: matrix.loadIgnoredUsers()
    }

    SilicaListView {
        id: listView

        anchors.fill: parent
        model: page.users

        header: Column {
            width: listView.width

            PageHeader {
                title: qsTr("Ignored users")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("The list belongs to your account: the server stops delivering these people's messages, in every client. Tap somebody to stop ignoring them.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                textFormat: Text.PlainText
                visible: matrix.lastError.length > 0
                text: matrix.lastError
            }
        }

        delegate: ListItem {
            id: row

            contentHeight: Theme.itemSizeSmall

            onClicked: {
                var userId = modelData
                page.activeRemorse = Remorse.popupAction(
                            page, qsTr("No longer ignoring"), function() {
                                page.activeRemorse = null
                                if (page.status !== PageStatus.Active
                                        || !Qt.application.active) {
                                    return
                                }
                                matrix.setMemberIgnored(userId, false)
                            })
            }

            Label {
                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                truncationMode: TruncationMode.Fade
                color: row.highlighted ? Theme.highlightColor : Theme.primaryColor
                textFormat: Text.PlainText
                text: modelData
            }
        }

        ViewPlaceholder {
            enabled: listView.count === 0
            text: qsTr("Nobody is ignored")
            hintText: qsTr("You can ignore somebody from their profile.")
        }

        VerticalScrollDecorator { }
    }
}
