import QtQuick 2.0
import Sailfish.Silica 1.0

// Invite someone into a room by their Matrix ID. The invitation lands in their
// client; nothing changes here until they accept, and the member list shows
// them as invited in the meantime.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    property string roomId
    property string roomName

    canAccept: userField.text.trim().length > 3

    onAccepted: matrix.inviteToRoom(roomId, userField.text)

    // In landscape the dialog is barely taller than its header, so the
    // field below it has to be reachable by scrolling — on the wide
    // device it would otherwise sit behind the keyboard.
    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            DialogHeader {
                acceptText: qsTr("Invite")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.secondaryColor
                textFormat: Text.PlainText
                visible: dialog.roomName.length > 0
                text: dialog.roomName
            }

            TextField {
                id: userField

                width: parent.width
                label: qsTr("User ID")
                placeholderText: "@name:server"
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                focus: true
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("They appear in the member list as invited until they accept.")
            }
        }
    }
}
