import QtQuick 2.0
import Sailfish.Silica 1.0

// Shown before a message goes into an encrypted room where a recipient still
// has devices the user never verified. Accepting sends anyway; the switch
// remembers that choice per user so the warning does not return for them.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    // Recipients to warn about: [{ userId, name, devices }].
    property var users: []

    canAccept: true

    onAccepted: {
        // Persist the decision only when asked, and only for the users this
        // dialog actually warned about.
        if (rememberSwitch.checked) {
            for (var i = 0; i < users.length; i++) {
                matrix.trustRecipient(users[i].userId)
            }
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            DialogHeader {
                acceptText: qsTr("Send anyway")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.highlightColor
                font.pixelSize: Theme.fontSizeLarge
                text: qsTr("Unverified devices")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.secondaryHighlightColor
                font.pixelSize: Theme.fontSizeSmall
                text: qsTr("The message will be encrypted for devices you have not verified. Verify them for real certainty, or send anyway.")
            }

            Column {
                width: parent.width

                Repeater {
                    model: dialog.users

                    Label {
                        x: Theme.horizontalPageMargin
                        width: parent.width - 2 * Theme.horizontalPageMargin
                        wrapMode: Text.Wrap
                        textFormat: Text.PlainText
                        text: (modelData.name && modelData.name.length > 0
                               ? modelData.name : modelData.userId)
                              + " — "
                              + qsTr("%n unverified device(s)", "", modelData.devices)
                    }
                }
            }

            TextSwitch {
                id: rememberSwitch

                text: dialog.users.length === 1
                      ? qsTr("Do not warn about this user again")
                      : qsTr("Do not warn about these users again")
                description: qsTr("Applies until you verify or clear it.")
            }
        }
    }
}
