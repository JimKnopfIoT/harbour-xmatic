import QtQuick 2.0
import Sailfish.Silica 1.0
import "Emoji.js" as Emoji

// Picking a reaction for one message.
//
// A reaction is not an icon set: Matrix carries it as a string, usually one
// emoji character, and whatever another client sent has to be drawn as it
// arrived. So these are characters, not images - a picture is only what the
// user put into the app's data directory, looked up from the character.
//
// The grid is the Unicode set in Unicode's own order and groups (Emoji.js),
// the first group being the common handful. Anything the set does not have -
// a word, a rare sequence - is typed into the field, which the system
// keyboard fills.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    property string eventId
    /// The reaction the user settled on, read by whoever pushed this.
    property string key: ""

    /// Index into Emoji.groups.
    property int group: 0

    /// Picking for the message being written rather than for a reaction. The
    /// accept text is then Silica's own, which already says "accept" in every
    /// language the app ships - "React" would be the wrong word and a new
    /// string in twenty-nine files for a button that a tap on an emoji makes
    /// unnecessary anyway.
    property bool forMessage: false

    canAccept: key.length > 0

    Column {
        id: head

        anchors { left: parent.left; right: parent.right; top: parent.top }

        DialogHeader {
            acceptText: dialog.forMessage ? defaultAcceptText : qsTr("React")
        }

        TextField {
            width: parent.width
            label: qsTr("Something else")
            placeholderText: qsTr("Something else")
            inputMethodHints: Qt.ImhNoPredictiveText
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: {
                if (text.trim().length > 0) {
                    dialog.key = text.trim()
                    dialog.accept()
                }
            }
            onTextChanged: dialog.key = text.trim()
        }

        // One row of groups. It scrolls sideways because ten of them do not
        // fit a portrait screen, and it carries no labels: a group's own
        // emoji says what it is in every language.
        SilicaListView {
            id: groups

            width: parent.width
            height: Theme.itemSizeSmall
            orientation: ListView.Horizontal
            currentIndex: -1
            model: Emoji.groups.length
            clip: true

            delegate: BackgroundItem {
                width: Theme.itemSizeSmall
                height: groups.height
                highlighted: down || dialog.group === index

                onClicked: {
                    dialog.group = index
                    grid.positionViewAtBeginning()
                }

                EmojiItem {
                    anchors.centerIn: parent
                    character: Emoji.groups[index].icon
                    size: Theme.iconSizeSmall
                }
            }
        }
    }

    SilicaGridView {
        id: grid

        anchors {
            left: parent.left
            right: parent.right
            top: head.bottom
            bottom: parent.bottom
        }
        clip: true
        // Qt gives row 0 the focus unless this says otherwise, and that
        // focus is taken from the field above on every group change.
        currentIndex: -1

        readonly property int columns: Math.max(4, Math.floor(width / Theme.itemSizeSmall))

        cellWidth: Math.floor(width / columns)
        cellHeight: cellWidth
        model: Emoji.groups[dialog.group].items

        delegate: BackgroundItem {
            width: grid.cellWidth
            height: grid.cellHeight
            highlighted: down || dialog.key === modelData

            onClicked: {
                dialog.key = modelData
                dialog.accept()
            }

            EmojiItem {
                anchors.centerIn: parent
                character: modelData
                size: Theme.iconSizeMedium
            }
        }

        VerticalScrollDecorator {}
    }
}
