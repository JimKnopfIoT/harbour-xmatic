import QtQuick 2.0
import Sailfish.Silica 1.0

// Session file in an unknown format, most likely from a newer build.
// The key opened it: no secrets help, no login.
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
                title: qsTr("Newer version needed")
                description: qsTr("Matrix for Sailfish OS")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                // "Most likely": a damaged envelope reads the same.
                text: qsTr("Your session was saved in a format this version does not know, most likely by a newer version of xmatic.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                // No server request from here: the device stays listed, its keys go.
                text: qsTr("If a newer version of xmatic was installed before, installing it again opens everything as before. Signing out instead deletes the data and keys this device holds.")
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Try again")
                onClicked: matrix.restoreSession()
            }
        }

        VerticalScrollDecorator { }
    }
}
