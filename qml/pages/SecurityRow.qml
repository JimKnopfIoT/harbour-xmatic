import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// One line of the security status: lamp, name, and one sentence saying what
// the colour means. The sentence is the point — a coloured dot on its own
// tells somebody that something is wrong, not what to do about it.
Column {
    id: row

    property string label
    property string level: "unknown"
    property string detail

    width: parent.width
    spacing: Theme.paddingSmall
    visible: level !== SecurityStatus.UNKNOWN

    Row {
        x: Theme.horizontalPageMargin
        width: parent.width - 2 * Theme.horizontalPageMargin
        spacing: Theme.paddingMedium

        SecurityLamp {
            level: row.level
            anchors.verticalCenter: parent.verticalCenter
        }

        Label {
            width: parent.width - Theme.iconSizeExtraSmall / 2 - Theme.paddingMedium
            wrapMode: Text.Wrap
            color: SecurityStatus.color(row.level, Theme,
                                        Theme.colorScheme === Theme.LightOnDark)
            text: row.label
        }
    }

    Label {
        x: Theme.horizontalPageMargin + Theme.iconSizeExtraSmall / 2 + Theme.paddingMedium
        width: parent.width - x - Theme.horizontalPageMargin
        wrapMode: Text.Wrap
        font.pixelSize: Theme.fontSizeExtraSmall
        color: Theme.secondaryColor
        text: row.detail
    }
}
