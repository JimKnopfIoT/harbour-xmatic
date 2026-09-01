import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// Shown once per app run when something about this device's security is not
// green — and never when everything is. It leads rather than blocks: "Later"
// is always there, because a hard gate strands whoever is offline or sitting
// somewhere without their recovery key, and forcing a sign-out on somebody who
// cannot unlock the backup afterwards is exactly the shape of the 0.16–0.18
// data loss.
//
// The frame is orange and static. Red is for what the system cannot do at all;
// everything reachable from here is a fault the user can clear.
Page {
    id: page

    allowedOrientations: Orientation.All

    readonly property string level: SecurityStatus.overall(matrix)
    readonly property bool allGreen: level === SecurityStatus.GREEN

    // Orange or green, never red. The frame is the attention-getter and it
    // does that job at one colour; turning it red as well would be shouting
    // twice for the same thing. Which of the four lines is a fault and which
    // is missing outright is said by the lamps beside them, where it is
    // information rather than alarm.
    //
    // Green once everything is: the page can be standing open while the
    // encryption page underneath it settles the last line, and a page framed
    // in orange over four green lamps contradicts itself. Reported from the
    // field, with the screenshot to go with it.
    readonly property color frameColor:
        SecurityStatus.color(page.allGreen ? SecurityStatus.GREEN
                                           : SecurityStatus.ORANGE,
                             Theme, Theme.colorScheme === Theme.LightOnDark)

    // Silica's PageHeader keeps clear of a camera cutout by itself; a strip
    // drawn by this app does not, and this frame ran along the very top edge -
    // straight under the notch. The same lesson 0.25.1 learned on the room's
    // own header, in a new place. Portrait only, as Silica has it: in landscape
    // the cutout is at the side, and on a phone without one this is zero.
    readonly property real topInset: orientation === Orientation.Portrait
                                     ? Screen.topCutout.height : 0

    Component.onCompleted: {
        matrix.refreshEncryptionStatus()
        matrix.refreshStorageStatus()
    }

    // Leaves as soon as there is nothing left to say. Entering the recovery key
    // on the encryption page turns every line green while this page is still
    // underneath, and a page that reports a solved problem teaches people to
    // tap it away without reading.
    //
    // Both signals, because that is the case it was written for and the case
    // it missed: the key is entered one page further in, so the state goes
    // green while this page is `Inactive` and the pop was refused. Coming back
    // then landed on the alarm the user had just cleared.
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

        // The frame encloses the content, not the screen - and that is the
        // whole point of it.
        //
        // Anchored to the page it drew a closed rectangle wherever the reader
        // stood, which says "this is all of it": in landscape the action at the
        // bottom sat below the fold and looked like nothing worth scrolling
        // for. Sized to the content it scrolls along, so its lower edge is
        // simply absent until the end is reached, and an unclosed frame says
        // there is more. Reported from the field on a landscape device.
        //
        // Behind the content and not on top: it is transparent but for its
        // border, and nothing that only decorates should sit over what it
        // decorates.
        Rectangle {
            z: -1
            // Never shorter than the screen. Sized to the content alone it
            // stopped where the text stopped and left a third of the display
            // empty below it, which reads as a broken layout rather than as a
            // frame. Longer than the screen only where the content really is.
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

            // The two ways out of an unverified device, in the order they
            // cost. A second device of one's own settles cross-signing in a
            // minute and needs nothing written down; the recovery key settles
            // three lines at once but has to be at hand.
            //
            // Offered first for a reason from the field: a new user arrived
            // here with his other device beside him, found only "enter your
            // recovery key", and went looking for a key he did not need. The
            // page knew the shorter way existed and did not say so.
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
                // What the button offers has to match what is actually
                // missing. A backup on the server with no recovery set up is a
                // real state, and offering "enter your key" there points at
                // something that does not exist.
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
