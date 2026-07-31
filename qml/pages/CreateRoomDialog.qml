import QtQuick 2.0
import Sailfish.Silica 1.0

// Create a room. Unlike a space this is a room with messages; unlike a direct
// chat it has a name and can hold any number of people.
//
// Both switches decide something that cannot be changed the same way later:
// encryption can only be turned on, never off, and a room's directory listing
// is a separate setting this app does not edit. Hence the hints below them.
Dialog {
    id: dialog

    canAccept: nameField.text.trim().length > 0

    onAccepted: matrix.createRoom(nameField.text, encryptionSwitch.checked,
                                  publicSwitch.checked)

    Column {
        width: parent.width
        spacing: Theme.paddingMedium

        DialogHeader {
            acceptText: qsTr("Create room")
        }

        TextField {
            id: nameField

            width: parent.width
            label: qsTr("Room name")
            placeholderText: qsTr("Room name")
            focus: true
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: dialog.accept()
        }

        TextSwitch {
            id: publicSwitch

            text: qsTr("Public room")
            description: qsTr("Listed in your homeserver's room directory, and anyone who finds it can join. Off means invitation only.")
        }

        TextSwitch {
            id: encryptionSwitch

            checked: true
            text: qsTr("End-to-end encryption")
            description: publicSwitch.checked
                         ? qsTr("Unusual for a public room: everyone joining later reads along from their join onwards, and nothing before it.")
                         : qsTr("Can only be decided now — encryption cannot be turned off again later.")
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryColor
            text: qsTr("The room opens right away. Invite people from its pulldown menu.")
        }
    }
}
