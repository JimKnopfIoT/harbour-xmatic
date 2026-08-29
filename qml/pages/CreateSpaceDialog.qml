import QtQuick 2.0
import Sailfish.Silica 1.0

// Create a new, empty space. A space is a room that groups other rooms and
// carries no messages of its own; once created it appears in the overview and
// rooms can be organised under it.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    // The word being typed is not in `text` yet: Sailfish's keyboard holds it
    // in the input method's preedit until it is committed. Without this the
    // accept button stays grey over a name that is plainly on screen, and a
    // one-word name arrives empty. Committing on acceptance is harmless even
    // where the toolkit already does it.
    canAccept: nameField.text.trim().length > 0 || nameField.inputMethodComposing

    onAccepted: {
        Qt.inputMethod.commit()
        matrix.createSpace(nameField.text)
    }

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
