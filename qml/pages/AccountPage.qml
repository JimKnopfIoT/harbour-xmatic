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

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Save name")
                enabled: nameField.text !== matrix.profileName
                onClicked: {
                    matrix.setDisplayName(nameField.text)
                    nameField.focus = false
                }
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: matrix.profileAvatar.length > 0
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

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Appearance")
                onClicked: pageStack.push(Qt.resolvedUrl("AppearancePage.qml"))
            }

            TextSwitch {
                text: qsTr("Message text in notifications")
                description: qsTr("Off, a notification says only how many messages arrived. On, it shows the latest message — also on the lock screen.")
                checked: matrix.notificationPreview
                automaticCheck: false
                onClicked: matrix.notificationPreview = !matrix.notificationPreview
            }

            TextSwitch {
                text: qsTr("Tappable web links")
                description: qsTr("On, a link in a message opens the browser when tapped. Off, links stay plain text.")
                checked: matrix.clickableLinks
                automaticCheck: false
                onClicked: matrix.clickableLinks = !matrix.clickableLinks
            }

            Item {
                width: 1
                height: Theme.paddingLarge
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
