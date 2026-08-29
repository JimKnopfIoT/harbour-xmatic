import QtQuick 2.0
import Sailfish.Silica 1.0

// The session and the stores are on disk, encrypted, and the key that opens
// them was not available when the app started — the device's secrets storage
// is bound to the device lock and hands the key out only after its own
// authentication. This page is deliberately not the login page: signing in
// here would create a new device and clear the store the key still protects.
// Every state needs a visible action, so the retry is a button and the way
// out for a key that is really gone is in the pull-down, behind a dialog.
Page {
    id: page

    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        PullDownMenu {
            MenuItem {
                text: qsTr("Sign out and delete local data")
                onClicked: pageStack.push(Qt.resolvedUrl("LogoutDialog.qml"))
            }
        }

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingLarge

            PageHeader {
                title: qsTr("Locked")
                description: qsTr("Matrix for Sailfish OS")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: qsTr("Your session is stored encrypted, and the key was not available when xmatic started.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("The key lives in the device's secrets storage. Try again and confirm the system's request; the approval lasts until the next restart of the device.")
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Try again")
                enabled: !matrix.busy
                onClicked: matrix.retryUnlock()
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: matrix.busy
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                visible: matrix.lastError.length > 0
                text: matrix.lastError
            }
        }

        VerticalScrollDecorator { }
    }
}
