import QtQuick 2.0
import Sailfish.Silica 1.0

// Who can be mentioned, offered while `@` is being typed. One scrolling row of
// names, not a list: a context menu's worth of rows has no place in landscape,
// and the row grows the composer upward instead of covering the text.
Item {
    id: picker

    /// Whose composer this belongs to. The core answers with the room it was
    /// asked about, so two open composers cannot take each other's list.
    property string roomId: ""
    /// Whether a mention is being typed. Deliberately not the field's focus: a
    /// press on this row is a press outside the field, Silica takes the focus
    /// for it, and the row would collapse under the finger.
    property bool armed: false

    readonly property var candidates: matrix.mentions.roomId === picker.roomId
                                      ? matrix.mentions.candidates : []
    readonly property bool open: armed && candidates.length > 0

    signal picked(string userId, string displayName)
    /// Emitted on the press, before the field loses its focus to it.
    signal keepKeyboardRequested()

    width: parent.width
    height: open ? Theme.itemSizeSmall + Theme.paddingSmall : 0
    visible: height > 0
    clip: true

    Behavior on height { NumberAnimation { duration: 100 } }

    Flickable {
        anchors.fill: parent
        // The margin is the Row's offset, not padding: those properties came
        // with a later QtQuick than this file may import.
        contentWidth: names.width + 2 * Theme.horizontalPageMargin
        flickableDirection: Flickable.HorizontalFlick

        Row {
            id: names

            x: Theme.horizontalPageMargin
            height: parent.height
            spacing: Theme.paddingLarge

            Repeater {
                model: picker.candidates

                // A row of its own per candidate: the avatar next to the name,
                // the whole thing one target.
                MouseArea {
                    id: chip

                    height: names.height
                    width: entry.width

                    onPressed: picker.keepKeyboardRequested()
                    onClicked: picker.picked(modelData.userId, modelData.displayName)

                    Row {
                        id: entry

                        anchors.verticalCenter: parent.verticalCenter
                        spacing: Theme.paddingSmall

                        Avatar {
                            anchors.verticalCenter: parent.verticalCenter
                            size: Theme.iconSizeSmall
                            source: modelData.avatar || ""
                            name: modelData.label
                        }

                        Label {
                            anchors.verticalCenter: parent.verticalCenter
                            font.pixelSize: Theme.fontSizeSmall
                            color: chip.pressed ? Theme.highlightColor
                                                : Theme.primaryColor
                            textFormat: Text.PlainText
                            text: modelData.label
                        }
                    }
                }
            }
        }
    }
}
