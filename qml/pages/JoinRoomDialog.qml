import QtQuick 2.0
import Sailfish.Silica 1.0

// Join a room by its address. Public rooms are advertised as "#name:server",
// and the server part is what lets the homeserver find a room it has never
// seen before.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    /// Filled in when a tapped link brought the user here.
    property string prefill: ""

    canAccept: aliasField.text.trim().length > 2

    onAccepted: matrix.joinRoomByAlias(aliasField.text)

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
                acceptText: qsTr("Join")
            }

            TextField {
                id: aliasField

                width: parent.width
                text: dialog.prefill
                label: qsTr("Room address")
                placeholderText: "#room:server"
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
                text: qsTr("The room appears in the list once the server has answered. Joining a large public room can take a moment.")
            }
        }
    }
}
