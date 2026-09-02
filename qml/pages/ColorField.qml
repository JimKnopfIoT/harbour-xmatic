import QtQuick 2.0
import Sailfish.Silica 1.0

// A free colour choice from plain gradients: the platform's own picker differs
// between the OS releases this app spans. `chosen` is "#rrggbb".
Column {
    id: root

    /// The colour under the marker, in "#rrggbb" form.
    property string chosen: "#3cb44b"

    /// Raised for every user change — fingertip, hex code or slider.
    signal edited(string colour)

    // Marker position in the spectrum's coordinates; negative while the
    // marker sits on the grey ramp instead.
    property real markerX: -1
    property real markerY: -1
    property real greyX: -1

    // The string does not expose channels; the colour type does.
    readonly property color chosenColor: chosen

    width: parent.width
    spacing: Theme.paddingMedium

    /// Shows `colorString` without reporting it as an edit.
    function setColor(colorString) {
        chosen = toHex(colorString)
        placeMarker(chosen)
        placeSliders()
    }

    /// A Silica slider stops following its binding once dragged, so a colour set
    /// from outside has to move the three of them by hand.
    function placeSliders() {
        for (var i = 0; i < channels.count; i++) {
            var slider = channels.itemAt(i)
            if (!slider) {
                continue
            }
            slider.value = Math.round(255 * (i === 0 ? chosenColor.r
                                           : i === 1 ? chosenColor.g
                                                     : chosenColor.b))
        }
    }

    function apply(colorString) {
        chosen = toHex(colorString)
        edited(chosen)
    }

    function toHex(colorString) {
        // Normalises any colour notation and drops the alpha nibble a QML
        // colour stringifies with.
        var s = String(Qt.darker(colorString, 1.0))
        return s.length === 9 ? "#" + s.substring(3) : s
    }

    /// Places the marker where `colorString` lives.
    function placeMarker(colorString) {
        var c = Qt.darker(colorString, 1.0)
        var r = c.r, g = c.g, b = c.b
        var max = Math.max(r, g, b), min = Math.min(r, g, b)
        var l = (max + min) / 2
        if (max === min) {
            // A grey: it lives on the ramp.
            greyX = (1 - l) * greyRamp.width
            markerX = -1
            markerY = -1
            return
        }
        var d = max - min
        var h
        if (max === r) {
            h = ((g - b) / d + (g < b ? 6 : 0)) / 6
        } else if (max === g) {
            h = ((b - r) / d + 2) / 6
        } else {
            h = ((r - g) / d + 4) / 6
        }
        markerX = h * spectrum.width
        markerY = (1 - l) * spectrum.height
        greyX = -1
    }

    Component.onCompleted: setColor(chosen)

    Item {
        id: spectrum

        x: Theme.horizontalPageMargin
        width: parent.width - 2 * Theme.horizontalPageMargin
        height: Math.min(Screen.height * 0.4, width)

        // The hue runs sideways: a vertical gradient turned on its side, the
        // one gradient direction Qt Quick can draw without effects.
        Rectangle {
            width: parent.height
            height: parent.width
            anchors.centerIn: parent
            rotation: -90
            gradient: Gradient {
                GradientStop { position: 0.000; color: "#ff0000" }
                GradientStop { position: 0.167; color: "#ffff00" }
                GradientStop { position: 0.333; color: "#00ff00" }
                GradientStop { position: 0.500; color: "#00ffff" }
                GradientStop { position: 0.667; color: "#0000ff" }
                GradientStop { position: 0.833; color: "#ff00ff" }
                GradientStop { position: 1.000; color: "#ff0000" }
            }
        }

        // Lightness runs down: white over the top half, black over the
        // bottom, the pure hue left untouched along the middle.
        Rectangle {
            anchors.fill: parent
            gradient: Gradient {
                GradientStop { position: 0.0; color: "#ffffffff" }
                GradientStop { position: 0.5; color: "#00ffffff" }
                GradientStop { position: 0.5; color: "#00000000" }
                GradientStop { position: 1.0; color: "#ff000000" }
            }
        }

        MouseArea {
            anchors.fill: parent
            onPressed: pick(mouse.x, mouse.y)
            onPositionChanged: pick(mouse.x, mouse.y)

            function pick(mx, my) {
                var px = Math.max(0, Math.min(spectrum.width, mx))
                var py = Math.max(0, Math.min(spectrum.height, my))
                root.markerX = px
                root.markerY = py
                root.greyX = -1
                root.apply(Qt.hsla(px / spectrum.width, 1.0,
                                   1.0 - py / spectrum.height, 1.0))
            }
        }

        Rectangle {
            visible: root.markerX >= 0
            x: root.markerX - width / 2
            y: root.markerY - height / 2
            width: Theme.iconSizeSmall
            height: width
            radius: width / 2
            color: "transparent"
            border.color: "white"
            border.width: 3

            Rectangle {
                anchors.fill: parent
                anchors.margins: -1
                radius: width / 2
                color: "transparent"
                border.color: "black"
                border.width: 1
            }
        }
    }

    // Pure white, grey and black live nowhere in the spectrum above, so they
    // get their own ramp.
    Item {
        id: greyRamp

        x: Theme.horizontalPageMargin
        width: parent.width - 2 * Theme.horizontalPageMargin
        height: Theme.itemSizeExtraSmall / 2

        Rectangle {
            width: parent.height
            height: parent.width
            anchors.centerIn: parent
            rotation: -90
            gradient: Gradient {
                GradientStop { position: 0.0; color: "#000000" }
                GradientStop { position: 1.0; color: "#ffffff" }
            }
        }

        MouseArea {
            anchors.fill: parent
            onPressed: pick(mouse.x)
            onPositionChanged: pick(mouse.x)

            function pick(mx) {
                var px = Math.max(0, Math.min(greyRamp.width, mx))
                root.greyX = px
                root.markerX = -1
                root.markerY = -1
                root.apply(Qt.hsla(0, 0, 1.0 - px / greyRamp.width, 1.0))
            }
        }

        Rectangle {
            visible: root.greyX >= 0
            x: root.greyX - width / 2
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.iconSizeSmall
            height: width
            radius: width / 2
            color: "transparent"
            border.color: "white"
            border.width: 3

            Rectangle {
                anchors.fill: parent
                anchors.margins: -1
                radius: width / 2
                color: "transparent"
                border.color: "black"
                border.width: 1
            }
        }
    }

    // What the fingertip picked, big enough to judge — and the same colour
    // as a hex code, readable and typeable.
    Row {
        x: Theme.horizontalPageMargin
        width: parent.width - 2 * Theme.horizontalPageMargin
        spacing: Theme.paddingMedium

        Rectangle {
            width: parent.width / 2 - Theme.paddingMedium / 2
            height: hexField.height - Theme.paddingLarge
            radius: Theme.paddingSmall / 2
            color: root.chosen
            border.color: Theme.primaryColor
            border.width: 1
        }

        TextField {
            id: hexField

            width: parent.width / 2 - Theme.paddingMedium / 2
            label: qsTr("Hex code")
            text: root.chosen
            inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
            // Applied while typing, once the text is a whole colour; the
            // marker and the sliders follow.
            onTextChanged: {
                var entered = text.trim()
                if (entered.length > 0 && entered.charAt(0) !== "#") {
                    entered = "#" + entered
                }
                if (/^#[0-9a-fA-F]{6}$/.test(entered)
                        && entered.toLowerCase() !== root.chosen.toLowerCase()) {
                    root.apply(entered.toLowerCase())
                    root.placeMarker(root.chosen)
                }
            }
            EnterKey.iconSource: "image://theme/icon-m-accept"
            EnterKey.onClicked: focus = false
        }
    }

    // One slider per channel, following `chosen`. The equality check in the
    // handler is what keeps that reflection from echoing back as an edit.
    Repeater {
        id: channels

        model: ["R", "G", "B"]

        Slider {
            width: root.width
            label: modelData
            minimumValue: 0
            maximumValue: 255
            stepSize: 1
            valueText: Math.round(value)
            value: Math.round(255 * (index === 0 ? root.chosenColor.r
                                   : index === 1 ? root.chosenColor.g
                                                 : root.chosenColor.b))
            onSliderValueChanged: {
                var channels = [Math.round(255 * root.chosenColor.r),
                                Math.round(255 * root.chosenColor.g),
                                Math.round(255 * root.chosenColor.b)]
                channels[index] = Math.round(sliderValue)
                var hex = "#" + channels.map(function(part) {
                    var piece = part.toString(16)
                    return piece.length === 1 ? "0" + piece : piece
                }).join("")
                if (hex.toLowerCase() !== root.chosen.toLowerCase()) {
                    root.apply(hex)
                    root.placeMarker(root.chosen)
                }
            }
        }
    }
}
