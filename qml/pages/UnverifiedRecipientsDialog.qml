import QtQuick 2.0
import Sailfish.Silica 1.0

// Shown before a message goes into an encrypted room with unchecked recipients.
// While this device is unverified nothing can be said about anyone else's.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    // Recipients to warn about: [{ userId, name, devices, reason }].
    property var users: []

    readonly property bool ownDevice: users.length === 1
                                      && users[0].reason === "ownDevice"

    function line(entry) {
        switch (entry.reason) {
        case "violation":
            return qsTr("keys changed since you verified them")
        case "identity":
            return qsTr("not verified")
        default:
            return qsTr("%n unverified device(s)", "", entry.devices)
        }
    }

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
                text: dialog.ownDevice ? qsTr("This device is not verified")
                                       : qsTr("Unverified recipients")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.secondaryHighlightColor
                font.pixelSize: Theme.fontSizeSmall
                text: dialog.ownDevice
                      ? qsTr("As long as this device is unverified, no other device can be shown as verified — not even one that is. Verify it under Account → Encryption, or send anyway.")
                      : qsTr("The message will be encrypted for recipients you have not verified. Verify them for real certainty, or send anyway.")
            }

            Column {
                width: parent.width

                Repeater {
                    model: dialog.ownDevice ? [] : dialog.users

                    Label {
                        x: Theme.horizontalPageMargin
                        width: parent.width - 2 * Theme.horizontalPageMargin
                        wrapMode: Text.Wrap
                        textFormat: Text.PlainText
                        text: (modelData.name && modelData.name.length > 0
                               ? modelData.name : modelData.userId)
                              + " — " + dialog.line(modelData)
                    }
                }
            }

            TextSwitch {
                id: rememberSwitch

                text: dialog.ownDevice
                      ? qsTr("Do not warn again")
                      : (dialog.users.length === 1
                         ? qsTr("Do not warn about this user again")
                         : qsTr("Do not warn about these users again"))
                description: qsTr("Applies until you verify or clear it.")
            }
        }
    }
}
