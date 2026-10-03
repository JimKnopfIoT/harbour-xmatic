import QtQuick 2.0
import Sailfish.Silica 1.0

// The held link, lit above its stepped-back message: the label's own text, clipped to the run.
// Placed at the label's position; `marks` come from LinkMarks.sweep.
Item {
    property Item label
    property var marks: []

    z: 1

    Repeater {
        model: marks

        Item {
            x: modelData.x
            y: modelData.y
            width: modelData.width
            height: modelData.height
            clip: true

            Rectangle {
                anchors.fill: parent
                radius: Theme.paddingSmall / 2
                color: Theme.rgba(Theme.highlightBackgroundColor,
                                  Theme.highlightBackgroundOpacity)
            }

            Label {
                x: -modelData.x
                y: -modelData.y
                width: label.width
                wrapMode: label.wrapMode
                maximumLineCount: label.maximumLineCount
                elide: label.elide
                textFormat: label.textFormat
                linkColor: label.linkColor
                font.pixelSize: label.font.pixelSize
                font.italic: label.font.italic
                color: label.color
                text: label.text
            }
        }
    }
}
