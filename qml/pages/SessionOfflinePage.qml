import QtQuick 2.0
import Sailfish.Silica 1.0

// Stored session, homeserver not reached. Not the login page: that would
// start a new device.
Page {
    id: page

    allowedOrientations: Orientation.All

    // Seconds to the next automatic try; doubles up to a minute.
    property int delay: 5

    function retry() {
        matrix.restoreSession()
        page.delay = Math.min(page.delay * 2, 60)
        retryTimer.restart()
        // Against a double tap.
        cooldown.restart()
    }

    Timer {
        id: retryTimer

        interval: page.delay * 1000
        running: true
        onTriggered: {
            // Background: re-arm, no request.
            if (Qt.application.active) {
                page.retry()
            } else {
                restart()
            }
        }
    }

    Timer {
        id: cooldown

        interval: 3000
    }

    Connections {
        target: Qt.application
        onActiveChanged: {
            // Three seconds apart; the core serialises the rest.
            if (Qt.application.active && !cooldown.running) {
                page.delay = 5
                page.retry()
            }
        }
    }

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
                title: qsTr("No connection")
                description: qsTr("Matrix for Sailfish OS")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                text: qsTr("Your homeserver could not be reached. You are still signed in, and nothing on this device has changed.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("xmatic keeps trying on its own. Do not sign in again: that would start a new device and cost the keys to your encrypted history.")
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Try again")
                enabled: !cooldown.running
                onClicked: {
                    page.delay = 5
                    page.retry()
                }
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: cooldown.running
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
