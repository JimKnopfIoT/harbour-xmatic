import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// Shown once per run when something is not green. It leads rather than
// blocks: a hard gate strands whoever is offline or without their recovery key.
Page {
    id: page

    allowedOrientations: Orientation.All

    readonly property string level: SecurityStatus.overall(matrix)
    readonly property bool allGreen: level === SecurityStatus.GREEN

    // Orange or green, never red: the frame is the attention-getter at one colour,
    // and the lamps say which line is a fault. Green once everything is.
    readonly property color frameColor:
        SecurityStatus.color(page.allGreen ? SecurityStatus.GREEN
                                           : SecurityStatus.ORANGE,
                             Theme, Theme.colorScheme === Theme.LightOnDark)

    // Silica keeps its own PageHeader clear of a camera cutout; a strip drawn here
    // does not. Portrait only, as Silica has it.
    readonly property real topInset: orientation === Orientation.Portrait
                                     ? Screen.topCutout.height : 0

    Component.onCompleted: {
        matrix.refreshEncryptionStatus()
        matrix.refreshStorageStatus()
    }

    // Leaves as soon as there is nothing left to say. Both signals: the key is
    // entered one page further in, so the state goes green while this is inactive.
    function settle() {
        if (page.allGreen && page.status === PageStatus.Active) {
            pageStack.pop()
        }
    }

    onLevelChanged: settle()
    onStatusChanged: settle()

    SilicaFlickable {
        id: flick

        anchors.fill: parent
        anchors.topMargin: page.topInset
        contentHeight: column.height + 2 * Theme.paddingLarge

        // The frame encloses the content, not the screen: anchored to the page it drew
        // a closed rectangle wherever the reader stood, which says "this is all of it".
        Rectangle {
            z: -1
            // Never shorter than the screen: sized to the content alone it left a third of
            // the display empty, which reads as a broken layout.
            width: flick.width
            height: Math.max(flick.height, flick.contentHeight)
            color: "transparent"
            border.width: Math.round(Theme.paddingSmall / 2)
            border.color: page.frameColor
            enabled: false
        }

        Column {
            id: column

            y: Theme.paddingLarge
            width: parent.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: qsTr("Security")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                // Green happens while this page is open, and for the moment
                // before it closes it must not still be claiming a fault.
                text: page.allGreen
                      ? qsTr("Everything on this device is in order.")
                      : qsTr("Something on this device is not in order yet. You can settle it now or later.")
            }

            SecurityRows { }

            Item {
                width: 1
                height: Theme.paddingMedium
            }

            // The two ways out of an unverified device, in the order they cost. A second
            // device settles it in a minute; the recovery key has to be at hand.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: SecurityStatus.crossSigningLevel(matrix) !== SecurityStatus.GREEN
                label: qsTr("Verify this device")
                enabled: !matrix.encryptionBusy
                onClicked: {
                    matrix.requestVerification("")
                    pageStack.push(Qt.resolvedUrl("VerificationPage.qml"))
                }
            }

            // The recovery key, below it: it settles more but asks for more.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: SecurityStatus.backupLevel(matrix) !== SecurityStatus.GREEN
                         || SecurityStatus.recoveryLevel(matrix) !== SecurityStatus.GREEN
                         || SecurityStatus.crossSigningLevel(matrix) !== SecurityStatus.GREEN
                // What the button offers has to match what is missing: a backup with no
                // recovery set up is a real state, and "enter your key" points at nothing.
                label: matrix.encryptionStatus.backupOnServer
                       && matrix.encryptionStatus.recovery !== "disabled"
                      ? qsTr("Enter recovery key")
                      : qsTr("Set up backup now")
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptionPage.qml"))
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: SecurityStatus.storageLevel(matrix) === SecurityStatus.ORANGE
                label: qsTr("Encrypt storage now")
                enabled: !matrix.encryptionBusy
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptStorageDialog.qml"))
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: SecurityStatus.storageLevel(matrix) === SecurityStatus.RED
                label: qsTr("Why is that")
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptionPage.qml"))
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: page.allGreen ? qsTr("Close") : qsTr("Later")
                onClicked: pageStack.pop()
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("You will find all of this again under Account › Encryption.")
            }
        }

        VerticalScrollDecorator { }
    }
}
