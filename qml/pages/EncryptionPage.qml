import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// Key backup and recovery.
//
// Room keys only ever reach the devices that existed when a message was sent.
// The backup is what lets a later device — or a reinstalled one — read the
// history at all, so this page is less about settings and more about not
// losing anything.
Page {
    id: page

    allowedOrientations: Orientation.All

    property string generatedKey: ""

    Component.onCompleted: {
        matrix.refreshEncryptionStatus()
        matrix.refreshStorageStatus()
    }

    // The recovery key unlocks the whole key backup — the same class of secret
    // as the login password, which the bridge wipes out of its send buffer. The
    // way back cannot be wiped that thoroughly: a QString handed into the QML
    // engine lives in its string pool and nothing can overwrite it there. What
    // is possible is not keeping it reachable once this page is gone, so both
    // the shown key and the typed one are dropped when the page is destroyed.
    // The residual is documented alongside the password's in
    // docs/PASSWORD-LOGIN.md.
    Component.onDestruction: {
        page.generatedKey = ""
        recoveryField.text = ""
    }

    Connections {
        target: matrix
        onRecoveryKeyReady: page.generatedKey = key
    }

    /// Sends the key and wipes the field. A recovery key on screen is a
    /// recovery key in a screenshot, in the task switcher and in whatever the
    /// input method kept.
    function useRecoveryKey(key) {
        matrix.recoverKeys(key)
        // A key that was pasted in is still sitting in the clipboard, where the
        // next application to ask for it can read it - and a recovery key opens
        // the whole backup. Cleared only when it is demonstrably this key: the
        // clipboard belongs to the user, and wiping something they put there
        // for another purpose would be taking it from them.
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

            // The only way an existing store changes side: it is created
            // encrypted or not at all. A sign-out clears it, the next sign-in
            // creates a fresh one under the key. Offered only where it can
            // actually work, and behind a dialog that says what it costs.
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

            // Only this device's own kin is verified from here. Verifying
            // another *person* used to sit right below, as a Matrix address to
            // type in — which asked the user for something they have never
            // seen written down, next to a button that means something else
            // entirely. It lives where the person does now: in the chat with
            // them, and in the chat list's menu for the rest.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Compares seven emoji with your other device. It needs that other device in front of you, and once both have confirmed, this one can read the shared room keys.")
            }

            // Named for what it does in the case that matters. Reported by a
            // new user: on a fresh device the other devices are already
            // verified, so "verify my other devices" reads as a no-op - and
            // this is the button that makes *this* one trusted. It still
            // serves the other direction, where an already verified device
            // starts the comparison; that reading is not lost by the name.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Verify this device")
                enabled: !matrix.encryptionBusy
                onClicked: {
                    matrix.requestVerification("")
                    pageStack.push(Qt.resolvedUrl("VerificationPage.qml"))
                }
            }

            // Only where there is something to unlock. An account with no
            // secret storage has no recovery key to enter, and this section
            // invited one anyway - the attempt then came back with the
            // library's own words, "the info about the secret key could not
            // have been found in the account data of the user", which is not a
            // sentence anybody should have to read. Shown while the state is
            // unknown: hiding the way in on a failed look is worse than
            // offering it in vain.
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
                // Sensitive: the flag that keeps the input method from
                // remembering what is typed. The password has carried it since
                // it existed; this field unlocks the whole key backup.
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
