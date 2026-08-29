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
    // Always orange, never red. The frame is the attention-getter and it does
    // that job at one colour; turning it red as well would be shouting twice
    // for the same thing. Which of the four lines is a fault and which is
    // missing outright is said by the lamps beside them, where it is
    // information rather than alarm.
    readonly property color frameColor:
        SecurityStatus.color(SecurityStatus.ORANGE, Theme,
                             Theme.colorScheme === Theme.LightOnDark)

    Component.onCompleted: {
        matrix.refreshEncryptionStatus()
        matrix.refreshStorageStatus()
    }

    // Leaves as soon as there is nothing left to say. Entering the recovery key
    // on the encryption page turns every line green while this page is still
    // underneath, and a page that reports a solved problem teaches people to
    // tap it away without reading.
    onLevelChanged: {
        if (level === SecurityStatus.GREEN && status === PageStatus.Active) {
            pageStack.pop()
        }
    }

    SilicaFlickable {
        id: flick

        anchors.fill: parent
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
            width: flick.width
            height: flick.contentHeight
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
                text: qsTr("Something on this device is not in order yet. You can settle it now or later.")
            }

            SecurityRows { }

            Item {
                width: 1
                height: Theme.paddingMedium
            }

            // The action that fits what is actually wrong. Only one is offered
            // at a time: a page with four buttons teaches nobody where to
            // start, and the recovery key settles three of the four lines at
            // once.
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: SecurityStatus.backupLevel(matrix) !== SecurityStatus.GREEN
                         || SecurityStatus.recoveryLevel(matrix) !== SecurityStatus.GREEN
                         || SecurityStatus.crossSigningLevel(matrix) !== SecurityStatus.GREEN
                text: matrix.encryptionStatus.backupOnServer
                      ? qsTr("Enter recovery key")
                      : qsTr("Set up backup now")
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptionPage.qml"))
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: SecurityStatus.storageLevel(matrix) === SecurityStatus.ORANGE
                text: qsTr("Encrypt storage now")
                enabled: !matrix.busy
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptStorageDialog.qml"))
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: SecurityStatus.storageLevel(matrix) === SecurityStatus.RED
                text: qsTr("Why is that")
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptionPage.qml"))
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Later")
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
