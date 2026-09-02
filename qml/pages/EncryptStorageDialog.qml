import QtQuick 2.0
import Sailfish.Silica 1.0

// A store created without encryption has no cipher row, so there is no
// in-place migration: sign out, which clears it, then sign in. Refused without a backup.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    property bool backupReady: matrix.encryptionStatus.backupOnServer

    canAccept: backupReady

    onAccepted: matrix.logout()

    Column {
        width: parent.width
        spacing: Theme.paddingLarge

        DialogHeader {
            acceptText: qsTr("Sign out and encrypt")
            cancelText: qsTr("Leave as it is")
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.primaryColor
            text: qsTr("An existing database cannot be encrypted in place. It has to be created anew: signing out deletes it, and the next sign-in creates it encrypted.")
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryColor
            visible: dialog.backupReady
            text: qsTr("You will need your recovery key afterwards to unlock the backup, and this device has to be verified again. Have the recovery key at hand before you continue.")
        }

        // Said before the sign-out, not discovered after it: on Sailfish 4 the browser
        // cannot finish an OAuth sign-in, which happened in the field test.
        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.errorColor
            visible: !matrix.browserLoginReliable
            text: qsTr("On this Sailfish version the browser cannot complete the sign-in. Come back in with “Sign in on another device” — that route works here.")
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.errorColor
            visible: !dialog.backupReady
            text: qsTr("Not possible yet: there is no key backup on the server. Signing out now would make every encrypted message on this device unreadable for good. Set up the backup first, then come back here.")
        }
    }
}
