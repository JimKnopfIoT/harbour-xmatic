import QtQuick 2.0
import Sailfish.Silica 1.0

// One confirmation in front of the actions that cannot be taken back, and it
// says out loud *what* it is about to act on.
//
// This exists because of a field report: a short tug at a pull-down menu had
// picked "Leave room" often enough by accident, and the remorse timer was no
// help — Silica fires a running remorse the moment its page deactivates
// (`RemorsePopup.qml`: "if the page is changed then execute immediately"), so
// going back to the room list *completed* the countdown instead of stopping
// it. The room was gone, and its name with it: nothing on screen had ever
// named the room that was being left, so it could not even be rejoined.
//
// The name is therefore the largest thing on this page. A remorse still runs
// after this dialog — that is the undo — but the decision is made here.
Dialog {
    id: dialog

    // The question, answered by accepting: "Really leave this room?"
    property string question
    // What the question is about — a room, a space, a person. Shown big.
    property string subject
    // What accepting costs, in one or two sentences. Optional.
    property string explanation
    // The accept label. Says the deed ("Leave"), never "OK".
    property string acceptLabel

    allowedOrientations: Orientation.All

    // Landscape on a small screen has almost no vertical room, so the content
    // scrolls rather than being cut off.
    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        VerticalScrollDecorator {}

        Column {
            id: column

            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                acceptText: dialog.acceptLabel
                cancelText: qsTr("Keep")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryColor
                textFormat: Text.PlainText
                text: dialog.question
            }

            // The whole point of the dialog. A room name can be long and can
            // contain anything a user typed, so it wraps and stays plain text.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeLarge
                color: Theme.highlightColor
                textFormat: Text.PlainText
                text: dialog.subject
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                visible: dialog.explanation.length > 0
                textFormat: Text.PlainText
                text: dialog.explanation
            }
        }
    }
}
