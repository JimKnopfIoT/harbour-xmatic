import QtQuick 2.0
import Sailfish.Silica 1.0

// Create a room. Everything is asked once because the server takes it once:
// encryption, address and federation are fixed for the room's lifetime.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    // The server part of the own address, for the preview: never the whole ID -
    // the preview explains what the room will be called, not who creates it.
    readonly property string homeServer: {
        var parts = matrix.userId.split(":")
        return parts.length > 1 ? parts[1] : ""
    }

    // The order the core expects; the combo box only knows its index.
    readonly property var historyKeys: ["world_readable", "shared", "invited", "joined"]

    // The word being typed sits in the input method's preedit, not in `text`:
    // without this the accept button stays grey over a name plainly on screen.
    canAccept: nameField.text.trim().length > 0 || nameField.inputMethodComposing

    onAccepted: {
        Qt.inputMethod.commit()
        var invited = []
        var entries = inviteField.text.split(/[,;\s]+/)
        for (var i = 0; i < entries.length; i++) {
            if (entries[i].length > 0) {
                invited.push(entries[i])
            }
        }

        matrix.createRoom({
            "name": nameField.text,
            "topic": topicField.text,
            "alias": publicSwitch.checked ? aliasField.text : "",
            "encrypted": encryptionSwitch.checked,
            "public": publicSwitch.checked,
            "historyVisibility": dialog.historyKeys[historyCombo.currentIndex],
            "invite": invited,
            "federate": !localSwitch.checked,
            "readOnly": readOnlySwitch.checked,
            "equalPower": !publicSwitch.checked && equalPowerSwitch.checked
        })
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

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
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: topicField.focus = true
            }

            TextField {
                id: topicField

                width: parent.width
                label: qsTr("Topic")
                placeholderText: qsTr("What the room is about")
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: focus = false
            }

            TextSwitch {
                id: publicSwitch

                text: qsTr("Public room")
                description: qsTr("Listed in your homeserver's room directory, and anyone who finds it can join. Off means invitation only.")
            }

            TextField {
                id: aliasField

                // The field belongs to the public case and to nothing else: a
                // private room cannot be published under an address.
                visible: publicSwitch.checked
                width: parent.width
                label: qsTr("Address")
                placeholderText: qsTr("Address")
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                description: text.trim().length > 0 && dialog.homeServer.length > 0
                             // The address people will type elsewhere, spelled out: the `#` and the server
                             // are added by the server, and nobody guesses that from a bare field.
                             ? qsTr("Reachable as %1").arg("#" + text.trim() + ":" + dialog.homeServer)
                             : qsTr("The name people can use to find the room, without # and without the server part. Optional.")
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: focus = false
            }

            ComboBox {
                id: historyCombo

                label: qsTr("Readable history")
                // What every preset sets on its own: members read everything,
                // including what was said before they arrived.
                currentIndex: 1

                menu: ContextMenu {
                    MenuItem { text: qsTr("Everyone, without joining") }
                    MenuItem { text: qsTr("Members, including earlier messages") }
                    MenuItem { text: qsTr("Members, from their invitation") }
                    MenuItem { text: qsTr("Members, from their join") }
                }
            }

            TextSwitch {
                id: encryptionSwitch

                checked: true
                text: qsTr("End-to-end encryption")
                description: publicSwitch.checked
                             ? qsTr("Unusual for a public room: everyone joining later reads along from their join onwards, and nothing before it.")
                             : qsTr("Can only be decided now — encryption cannot be turned off again later.")
            }

            TextSwitch {
                id: readOnlySwitch

                text: qsTr("Only moderators may write")
                description: qsTr("For an announcement room. Everyone else can read, but not write and not react.")
            }

            TextSwitch {
                id: equalPowerSwitch

                // Private rooms only: in a public one it would promote whoever happened to be
                // invited at creation and nobody who joins later.
                visible: !publicSwitch.checked
                text: qsTr("Invited people get my rights")
                description: qsTr("Everyone invited below starts as an administrator. Later members do not.")
            }

            TextSwitch {
                id: localSwitch

                text: qsTr("Keep on this server")
                description: qsTr("People on other servers cannot join, not even by invitation. Cannot be changed later.")
            }

            TextField {
                id: inviteField

                width: parent.width
                label: qsTr("Invite")
                placeholderText: qsTr("Matrix addresses, separated by commas")
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                description: qsTr("Invited as the room is created. You can invite more people later from the room's pulldown menu.")
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("The room opens right away.")
            }
        }
    }
}
