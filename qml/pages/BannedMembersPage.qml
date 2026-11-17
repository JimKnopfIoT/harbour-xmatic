import QtQuick 2.0
import Sailfish.Silica 1.0

// Who is banned from one room, by whom and why. Read-only without the power
// to ban; no diff follows a ban, matrix.bans writes each answer in.
Page {
    id: page

    property string roomId
    property string roomName
    property string actionError: ""

    // The list belongs to whichever room was loaded last.
    readonly property bool mine: matrix.bans.roomId === page.roomId

    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.bans.load(roomId)

    Connections {
        target: matrix.bans
        onFailed: {
            if (roomId === page.roomId) {
                page.actionError = message
            }
        }
        onBanned: {
            if (roomId === page.roomId) {
                page.actionError = ""
            }
        }
        onUnbanned: {
            if (roomId === page.roomId) {
                page.actionError = ""
            }
        }
    }

    SilicaListView {
        id: listView

        anchors.fill: parent
        model: page.mine ? matrix.bans.entries : []

        PullDownMenu {
            MenuItem {
                text: qsTr("Ban someone…")
                visible: page.mine && matrix.bans.canBan
                onClicked: pageStack.push(Qt.resolvedUrl("BanUserDialog.qml"), {
                                              roomId: page.roomId,
                                              roomName: page.roomName
                                          })
            }

            MenuItem {
                text: qsTranslate("MemberProfilePage", "Reload")
                onClicked: matrix.bans.load(page.roomId)
            }
        }

        header: Column {
            width: listView.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: qsTr("Banned people")
                description: page.roomName
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                visible: page.mine && matrix.bans.loaded && !matrix.bans.canBan
                text: qsTr("Your role in this room does not allow banning or lifting bans. The list is for reading only.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                textFormat: Text.PlainText
                visible: page.actionError.length > 0
                text: page.actionError
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                textFormat: Text.PlainText
                visible: page.mine && matrix.bans.loadError.length > 0
                text: matrix.bans.loadError
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.mine && matrix.bans.loadError.length > 0
                text: qsTranslate("SessionOfflinePage", "Try again")
                onClicked: matrix.bans.load(page.roomId)
            }
        }

        delegate: ListItem {
            id: row

            contentHeight: entry.height + 2 * Theme.paddingMedium
            menu: modelData.canUnban ? liftMenu : null

            onClicked: {
                if (modelData.canUnban) {
                    openMenu()
                }
            }

            Component {
                id: liftMenu

                ContextMenu {
                    MenuItem {
                        text: qsTranslate("MemberProfilePage", "Lift ban")
                        onClicked: matrix.bans.unban(page.roomId, modelData.userId)
                    }
                }
            }

            Column {
                id: entry

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
                    color: row.highlighted ? Theme.highlightColor : Theme.primaryColor
                    textFormat: Text.PlainText
                    text: modelData.displayName.length > 0 ? modelData.displayName
                                                           : modelData.userId
                }

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: row.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    textFormat: Text.PlainText
                    visible: modelData.displayName.length > 0
                    text: modelData.userId
                }

                Label {
                    width: parent.width
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: row.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    textFormat: Text.PlainText
                    visible: modelData.reason.length > 0
                    text: qsTr("Reason: %1").arg(modelData.reason)
                }

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: row.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    textFormat: Text.PlainText
                    visible: modelData.bannedBy.length > 0
                    text: qsTr("Banned by %1").arg(modelData.bannedByName.length > 0
                                                   ? modelData.bannedByName
                                                   : modelData.bannedBy)
                }
            }
        }

        ViewPlaceholder {
            enabled: page.mine && matrix.bans.loaded && listView.count === 0
            text: qsTr("Nobody is banned")
        }

        VerticalScrollDecorator { }
    }

    BusyIndicator {
        anchors.centerIn: parent
        size: BusyIndicatorSize.Large
        running: page.mine && ((matrix.bans.loading && !matrix.bans.loaded) || matrix.bans.busy)
    }
}
