import QtQuick 2.0
import Sailfish.Silica 1.0

// One of two tabs above the attachment picker. Takes its width as given: a
// wrapping row of these would collapse if it measured its own children.
BackgroundItem {
    id: tab

    property string label: ""
    property bool active: false

    height: Theme.itemSizeSmall

    Label {
        anchors.centerIn: parent
        font.pixelSize: Theme.fontSizeSmall
        color: tab.active ? Theme.highlightColor : Theme.secondaryColor
        text: tab.label
    }

    Rectangle {
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }
        height: Math.round(Theme.paddingSmall / 3)
        color: Theme.highlightColor
        visible: tab.active
    }
}
