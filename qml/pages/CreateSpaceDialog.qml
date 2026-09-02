import QtQuick 2.0
import Sailfish.Silica 1.0

// Create an empty space: a room that groups other rooms and carries no
// messages of its own.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    // The word being typed sits in the input method's preedit, not in `text`:
    // without this the accept button stays grey over a name plainly on screen.
    canAccept: nameField.text.trim().length > 0 || nameField.inputMethodComposing

    onAccepted: {
        Qt.inputMethod.commit()
        matrix.createSpace(nameField.text)
    }

    // In landscape the dialog is barely taller than its header, so the field has
    // to be reachable by scrolling.
    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            DialogHeader {
                acceptText: qsTr("Create space")
            }

            TextField {
                id: nameField

                width: parent.width
                label: qsTr("Space name")
                placeholderText: qsTr("Space name")
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
                text: qsTr("The space appears in the overview right away. It is private and holds no messages — it is a folder for rooms.")
            }
        }
    }
}
