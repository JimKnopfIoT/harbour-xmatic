import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// Key backup and recovery. Room keys only reach the devices that existed when
// a message was sent, so this is about not losing history.
Page {
    id: page

    allowedOrientations: Orientation.All

    property string generatedKey: ""

    Component.onCompleted: {
        matrix.refreshEncryptionStatus()
        matrix.refreshStorageStatus()
    }

    // The recovery key is the same class of secret as the password. A QString in
    // the QML engine cannot be wiped, so it is at least dropped with the page.
    Component.onDestruction: {
        // The copy button put it there and only this can take it back. Cleared
        // only where it is demonstrably ours - the clipboard is the user's.
        if (page.generatedKey.length > 0
                && Clipboard.hasText && Clipboard.text === page.generatedKey) {
            Clipboard.text = ""
        }
        page.generatedKey = ""
        recoveryField.text = ""
    }

    Connections {
        target: matrix
        onRecoveryKeyReady: page.generatedKey = key
    }

    /// Sends the key and wipes the field: a recovery key on screen is one in a
    /// screenshot, in the switcher and in whatever the input method kept.
    function useRecoveryKey(key) {
        matrix.recoverKeys(key)
        // A pasted key still sits in the clipboard, where the next app can read it.
        // Cleared only when it is demonstrably this key - the clipboard is the user's.
        if (Clipboard.hasText && Clipboard.text === key) {
            Clipboard.text = ""
        }
        recoveryField.text = ""
        recoveryField.focus = false
    }

    readonly property bool appForeground: Qt.application.active
    onAppForegroundChanged: {
        if (!appForeground) {
            recoveryField.text = ""
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: qsTr("Encryption")
            }

            SecurityRows { }

            // The only way an existing store changes side: it is created encrypted or not
            // at all. Offered where it can work, behind a dialog that says the cost.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: matrix.storageStatus.canEncrypt
                label: qsTr("Encrypt local storage")
                enabled: !matrix.encryptionBusy
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptStorageDialog.qml"))
            }

            SectionHeader {
                text: qsTr("Verify")
            }

            // Only this device's own kin is verified from here. Verifying a person lives
            // where the person does: in the chat with them.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Compares seven emoji with your other device. It needs that other device in front of you, and once both have confirmed, this one can read the shared room keys.")
            }

            // Named for the case that matters: on a fresh device the others are already
            // verified, and this is the button that makes *this* one trusted.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Verify this device")
                enabled: !matrix.encryptionBusy
                onClicked: {
                    matrix.requestVerification("")
                    pageStack.push(Qt.resolvedUrl("VerificationPage.qml"))
                }
            }

            // Only where there is something to unlock: an account without secret storage
            // has no key to enter. Shown while unknown - hiding on a failed look is worse.
            readonly property bool canUnlockBackup:
                matrix.encryptionStatus.recovery !== "disabled"

            SectionHeader {
                text: qsTr("Unlock backup")
                visible: column.canUnlockBackup
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                visible: column.canUnlockBackup
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Enter the recovery key from your other client. This device then fetches the room keys it is missing, and older messages become readable.")
            }

            TextField {
                id: recoveryField

                visible: column.canUnlockBackup

                width: parent.width
                label: qsTr("Recovery key")
                placeholderText: qsTr("Recovery key")
                // Sensitive: the flag that keeps the input method from remembering what is
                // typed. This field unlocks the whole key backup.
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                                  | Qt.ImhSensitiveData
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.useRecoveryKey(text)
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: column.canUnlockBackup
                label: qsTr("Unlock")
                enabled: !matrix.encryptionBusy && recoveryField.text.trim().length > 0
                onClicked: page.useRecoveryKey(recoveryField.text)
            }

            SectionHeader {
                text: qsTr("Set up backup")
                visible: !matrix.encryptionStatus.backupOnServer
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                visible: !matrix.encryptionStatus.backupOnServer
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Creates a backup of your room keys on the server, encrypted with a recovery key that only you hold. Without it, reinstalling loses every encrypted message.")
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: !matrix.encryptionStatus.backupOnServer
                label: qsTr("Set up backup")
                enabled: !matrix.encryptionBusy
                onClicked: matrix.enableKeyBackup()
            }

            // Shown once. There is no second chance, so it is deliberately
            // prominent and copyable.
            Column {
                width: parent.width
                spacing: Theme.paddingSmall
                visible: page.generatedKey.length > 0

                SectionHeader {
                    text: qsTr("Your recovery key")
                }

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    font.family: Theme.fontFamilyHeading
                    color: Theme.highlightColor
                    text: page.generatedKey
                }

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.errorColor
                    text: qsTr("Write this down now. It is shown only once and is not stored on this device.")
                }

                WrapButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    label: qsTr("Copy")
                    onClicked: Clipboard.text = page.generatedKey
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                visible: matrix.lastError.length > 0
                text: matrix.lastError
            }
        }

        VerticalScrollDecorator { }
    }
}
