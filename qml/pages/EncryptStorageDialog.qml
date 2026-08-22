import QtQuick 2.0
import Sailfish.Silica 1.0

// Turning an existing, unencrypted local store into an encrypted one.
//
// There is no in-place migration and there cannot be a cheap one: a store
// created without encryption has no cipher row, and opening it under a key
// would mint a fresh cipher and turn every value into garbage. What works is
// starting over — sign out, which clears the store, then sign in again, which
// creates it under the key.
//
// That means this dialog offers exactly the operation that cost users their
// history once before, only this time deliberately and with the one condition
// that makes it safe: the room keys must already be in a backup on the server.
// Without that, accepting is refused rather than warned about.
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
