import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0

// The signed-in account: identity, own profile (display name and avatar) and
// the entry points for encryption and signing out.
Page {
    id: page

    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.fetchProfile()

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: qsTr("Signed in")
            }

            DetailItem {
                label: qsTr("Account")
                value: matrix.userId
            }

            DetailItem {
                label: qsTr("Device")
                value: matrix.deviceId
            }

            DetailItem {
                label: qsTr("Core")
                value: matrix.coreVersion
            }

            // One direction only, and now it is the other one: a wrapping
            // button takes its width as given and lays the label out inside it,
            // so deriving that width from the children's implicitWidth is not
            // just forbidden but meaningless - a WrapButton carries no text of
            // its own, so its implicit width is the platform minimum and every
            // button on the page collapsed to it.
            readonly property real buttonWidth: Math.min(
                    Theme.buttonWidthLarge,
                    width - 2 * Theme.horizontalPageMargin)

            SectionHeader {
                text: qsTr("Profile")
            }

            TextField {
                id: nameField

                width: parent.width
                label: qsTr("Display name")
                placeholderText: qsTr("Display name")
                text: matrix.profileName
                EnterKey.iconSource: "image://theme/icon-m-accept"
                EnterKey.onClicked: {
                    matrix.setDisplayName(text)
                    focus = false
                }
            }

            WrapButton {
                id: saveNameButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                label: qsTr("Save name")
                enabled: nameField.text !== matrix.profileName
                onClicked: {
                    matrix.setDisplayName(nameField.text)
                    nameField.focus = false
                }
            }

            WrapButton {
                id: avatarButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                label: matrix.profileAvatar.length > 0
                      ? qsTr("Change avatar")
                      : qsTr("Set avatar")
                onClicked: pageStack.push(avatarPicker)
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                visible: matrix.profileAvatar.length > 0
                text: qsTr("An avatar is set. Other people see it next to your name.")
            }

            SectionHeader {
                text: qsTr("This app")
            }

            WrapButton {
                id: appearanceButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                label: qsTr("Appearance")
                onClicked: pageStack.push(Qt.resolvedUrl("AppearancePage.qml"))
            }

            WrapButton {
                id: privacyButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                label: qsTr("Privacy")
                onClicked: pageStack.push(Qt.resolvedUrl("PrivacyPage.qml"))
            }

            WrapButton {
                id: ignoredButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                label: qsTr("Ignored users")
                onClicked: pageStack.push(Qt.resolvedUrl("IgnoredUsersPage.qml"))
            }

            // The send warning offers "do not warn me about this user again",
            // and its own subtitle promises it holds "until you reset it" —
            // this is that reset. Kept as one button rather than a list: the
            // entries are addresses, and the list would be a second place
            // where they are on screen for no gain.
            WrapButton {
                id: resetWarningsButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                label: qsTr("Reset send warnings")
                onClicked: {
                    var count = matrix.resetRecipientWarnings()
                    resetWarningsHint.text = count > 0
                            ? qsTr("%n recipient(s) will warn again", "", count)
                            : qsTr("No suppressed warnings")
                    resetWarningsHint.opacity = 1.0
                    resetWarningsTimer.restart()
                }
            }

            Label {
                id: resetWarningsHint

                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                opacity: 0.0
                visible: opacity > 0
                Behavior on opacity { FadeAnimation { } }

                Timer {
                    id: resetWarningsTimer
                    interval: 3000
                    onTriggered: resetWarningsHint.opacity = 0.0
                }
            }

            ValueButton {
                label: qsTr("Language")
                value: {
                    var list = language.available
                    for (var i = 0; i < list.length; ++i) {
                        if (list[i].code === language.code) {
                            return list[i].name
                        }
                    }
                    return ""
                }
                onClicked: pageStack.push(Qt.resolvedUrl("LanguagePage.qml"))
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }

            // Said where somebody comes past it, not only where somebody goes
            // looking for it. The encryption page has stated this since the
            // stores could be encrypted at all, but a state one has to seek
            // out is a state nobody knows about - which is exactly what an
            // outside reader of this app pointed at. Only where a key could be
            // had: without secretsd there is nothing to act on, and a line
            // that offers nothing is a nag.
            BackgroundItem {
                width: parent.width
                height: unencryptedWarning.height + 2 * Theme.paddingMedium
                visible: !matrix.storageStatus.encrypted
                         && matrix.storageStatus.keyAvailable
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptionPage.qml"))

                Label {
                    id: unencryptedWarning

                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.errorColor
                    text: qsTr("Session and message database lie on this device unencrypted. Tap to encrypt them.")
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

        PullDownMenu {
            MenuItem {
                text: qsTr("Sign out")
                onClicked: pageStack.push(Qt.resolvedUrl("LogoutDialog.qml"))
            }

            MenuItem {
                text: qsTr("Encryption")
                onClicked: pageStack.push(Qt.resolvedUrl("EncryptionPage.qml"))
            }
        }

        VerticalScrollDecorator { }
    }

    Component {
        id: avatarPicker

        ImagePickerPage {
            onSelectedContentPropertiesChanged: {
                if (selectedContentProperties.filePath.length > 0) {
                    matrix.setAvatarFile(selectedContentProperties.filePath)
                }
            }
        }
    }
}
