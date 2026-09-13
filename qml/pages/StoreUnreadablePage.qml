import QtQuick 2.0
import Sailfish.Silica 1.0

// Stored data that could not be read back. Account and keys are fine, so not
// the login page: a sign-in over this store would cost the device.
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
                title: qsTr("Local data unreadable")
                description: qsTr("Matrix for Sailfish OS")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: qsTr("xmatic could not read back part of the data it keeps on this device. Your account, this device and its encryption keys are not affected.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Rebuilding deletes the rooms and messages stored on this device and loads them again from your homeserver. This device and its keys stay; messages that were not sent yet are lost.")
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Rebuild local data")
                enabled: !matrix.busy
                onClicked: matrix.rebuildLocalData()
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
