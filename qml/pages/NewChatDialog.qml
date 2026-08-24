import QtQuick 2.0
import Sailfish.Silica 1.0

// Start a conversation with someone by their Matrix ID.
//
// If a private room with exactly that person already exists it is reused —
// two direct rooms with the same contact would split the conversation.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    /// Filled in when a tapped link brought the user here.
    property string prefill: ""

    canAccept: userField.text.trim().length > 3

    onAccepted: matrix.startDirectChat(userField.text)

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
                acceptText: qsTr("Start chat")
            }

            TextField {
                id: userField

                width: parent.width
                text: dialog.prefill
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
                text: qsTr("The other side receives an invitation and the chat appears in your list right away. Messages are encrypted from the start.")
            }
        }
    }
}
