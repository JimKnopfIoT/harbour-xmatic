import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// A round indicator for one security level, or for the whole device.
//
// Static, not blinking: this appears on the pages the user sees every day, and
// a permanent animation reads as "the app is broken" rather than "act on this".
// Silica has no idiom for a blinking control either.
//
// Invisible while the state is unknown. Painting a colour over an unanswered
// question is worse than showing nothing — the app would be making a claim it
// cannot back.
Item {
    id: lamp

    /// "green", "orange", "red" or "unknown".
    property string level: "unknown"
    /// Set instead of `level` to show the device's overall state.
    property bool overall: false

    readonly property string shownLevel: overall ? SecurityStatus.overall(matrix) : level
    readonly property bool known: shownLevel !== SecurityStatus.UNKNOWN

    implicitWidth: Theme.iconSizeExtraSmall / 2
    implicitHeight: implicitWidth
    width: implicitWidth
    height: implicitHeight
    visible: known

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        color: SecurityStatus.color(lamp.shownLevel, Theme,
                                    Theme.colorScheme === Theme.LightOnDark)
        // A thin darker ring so the dot stays visible on an ambience whose
        // background happens to be close to the colour itself.
        border.width: Math.max(1, width / 12)
        border.color: Theme.rgba(Theme.primaryColor, 0.3)
    }
}
