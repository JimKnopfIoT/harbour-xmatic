import QtQuick 2.0
import Sailfish.Silica 1.0

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

    Component.onCompleted: matrix.refreshEncryptionStatus()

    Connections {
        target: matrix
        onRecoveryKeyReady: page.generatedKey = key
    }

    function verifyUser() {
        matrix.requestVerification(userField.text)
        pageStack.push(Qt.resolvedUrl("VerificationPage.qml"))
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

            DetailItem {
                label: qsTr("Backup")
                value: matrix.encryptionStatus.backupEnabled
                       ? qsTr("active")
                       : (matrix.encryptionStatus.backupOnServer
                          ? qsTr("exists, not unlocked")
                          : qsTr("not set up"))
            }

            DetailItem {
                label: qsTr("Recovery")
                value: {
                    switch (matrix.encryptionStatus.recovery) {
                    case "enabled": return qsTr("set up")
                    case "incomplete": return qsTr("incomplete")
                    case "disabled": return qsTr("not set up")
                    default: return qsTr("unknown")
                    }
                }
            }

            DetailItem {
                label: qsTr("Cross-signing")
                value: matrix.encryptionStatus.crossSigned ? qsTr("complete") : qsTr("incomplete")
            }

            SectionHeader {
                text: qsTr("Verify")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Verifying compares seven emoji with the other side. Between your own devices it also unlocks shared room keys.")
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Verify my other devices")
                enabled: !matrix.busy
                onClicked: {
                    matrix.requestVerification("")
                    pageStack.push(Qt.resolvedUrl("VerificationPage.qml"))
                }
            }

            TextField {
                id: userField

                width: parent.width
                label: qsTr("User ID")
                placeholderText: qsTr("@name:server")
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.verifyUser()
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Verify this user")
                enabled: !matrix.busy && userField.text.trim().length > 0
                onClicked: page.verifyUser()
            }

            SectionHeader {
                text: qsTr("Unlock backup")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Enter the recovery key from your other client. This device then fetches the room keys it is missing, and older messages become readable.")
            }

            TextField {
                id: recoveryField

                width: parent.width
                label: qsTr("Recovery key")
                placeholderText: qsTr("Recovery key")
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: matrix.recoverKeys(text)
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Unlock")
                enabled: !matrix.busy && recoveryField.text.trim().length > 0
                onClicked: matrix.recoverKeys(recoveryField.text)
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

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: !matrix.encryptionStatus.backupOnServer
                text: qsTr("Set up backup")
                enabled: !matrix.busy
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

                Button {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: qsTr("Copy")
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
