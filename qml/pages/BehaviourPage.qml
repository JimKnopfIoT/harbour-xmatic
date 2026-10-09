import QtQuick 2.0
import Sailfish.Silica 1.0

// How rooms open, how writing works, and which room events show. A hidden event
// stays in the model and takes no space, so switching back shows what it had.
Page {
    id: page

    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: qsTr("Behaviour")
            }

            SectionHeader {
                text: qsTr("Lists")
            }

            // Read once at start, so a change shows on the next one.
            TextSwitch {
                text: qsTr("Start with the space list")
                description: qsTr("Off, the app opens with the chat list. Either way the other list is one swipe sideways. Takes effect on the next start.")
                checked: settings.startPage === "spaces"
                automaticCheck: false
                onClicked: settings.startPage = checked ? "rooms" : "spaces"
            }

            ComboBox {
                width: parent.width
                label: qsTr("Chat list order")
                currentIndex: behaviour.roomOrder === "name" ? 1 : 0

                menu: ContextMenu {
                    MenuItem { text: qsTr("By activity") }
                    MenuItem { text: qsTr("By name") }
                }

                onCurrentIndexChanged: behaviour.roomOrder = currentIndex === 1 ? "name" : "activity"
            }

            TextSwitch {
                text: qsTr("Unread rooms first")
                description: qsTr("Favourites stay at the top and low priority at the bottom; inside each, rooms with unread messages come first.")
                checked: behaviour.unreadFirst
                automaticCheck: false
                onClicked: behaviour.unreadFirst = !behaviour.unreadFirst
            }

            ComboBox {
                width: parent.width
                label: qsTr("Space list order")
                currentIndex: behaviour.spaceOrder === "name" ? 1 : 0

                menu: ContextMenu {
                    MenuItem { text: qsTr("By activity") }
                    MenuItem { text: qsTr("By name") }
                }

                onCurrentIndexChanged: behaviour.spaceOrder = currentIndex === 1 ? "name" : "activity"
            }

            SectionHeader {
                text: qsTr("Reading")
            }

            // The line itself is not a setting - it costs nothing and appears only where
            // something is unread. What differs is where the room opens.
            TextSwitch {
                text: qsTr("Open a room where you stopped reading")
                description: qsTr("On, entering a room takes you to your last read message, with the new ones below it — and back to the place you were at if you left the room in the middle of its history. That place is kept until the app is closed. Off, the room opens at its newest message and the line marking where you stopped is found by scrolling up.")
                checked: settings.jumpToReadMarker
                automaticCheck: false
                onClicked: settings.jumpToReadMarker = !settings.jumpToReadMarker
            }

            SectionHeader {
                text: qsTr("Writing")
            }

            TextSwitch {
                text: qsTr("Send with the return key")
                description: qsTr("On, the return key sends the message; the arrow beside the field keeps working. A line break then comes from holding that arrow, or from shift and the return key on a hardware keyboard. Off, the return key makes a line break and only the arrow sends.")
                checked: settings.sendByEnter
                automaticCheck: false
                onClicked: settings.sendByEnter = !settings.sendByEnter
            }

            TextSwitch {
                text: qsTr("Hide the keyboard after sending")
                description: qsTr("On, the keyboard closes once a message is out and the conversation is back in full. Off, it stays up for the next one.")
                checked: settings.hideKeyboardOnSend
                automaticCheck: false
                onClicked: settings.hideKeyboardOnSend = !settings.hideKeyboardOnSend
            }

            SectionHeader {
                text: qsTr("Room events")
            }

            TextSwitch {
                text: qsTr("Show join and leave messages")
                description: qsTr("Invitations, removals and bans are always shown.")
                checked: behaviour.showJoinLeaves
                automaticCheck: false
                onClicked: behaviour.showJoinLeaves = !behaviour.showJoinLeaves
            }

            TextSwitch {
                text: qsTr("Show display name changes")
                checked: behaviour.showDisplayNameChanges
                automaticCheck: false
                onClicked: behaviour.showDisplayNameChanges = !behaviour.showDisplayNameChanges
            }

            TextSwitch {
                text: qsTr("Show profile picture changes")
                checked: behaviour.showAvatarChanges
                automaticCheck: false
                onClicked: behaviour.showAvatarChanges = !behaviour.showAvatarChanges
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }
    }
}
