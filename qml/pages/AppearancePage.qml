import QtQuick 2.0
import Sailfish.Silica 1.0

// The conversation's colours, chosen by the user. One spectrum serves every
// element: the selector underneath decides what the fingertip is colouring,
// the preview at the top uses the very expressions the conversation uses, so
// a choice shows its effect before it ever hits a chat — and the pull-down's
// reset is the way back out of an unreadable palette. Every element defaults
// to "follow the ambience", which reproduces the built-in look.
Page {
    id: page

    // What the spectrum is currently colouring.
    readonly property var elementKeys: ["otherBubble", "ownBubble", "name",
                                        "otherText", "ownText"]
    property string element: "otherBubble"

    // The stored value of the selected element, "" for "follow the
    // ambience". A ternary chain rather than a lookup so it stays reactive
    // to the settings object.
    readonly property string storedValue:
        element === "otherBubble" ? appearance.otherBubbleColor
        : element === "ownBubble" ? appearance.ownBubbleColor
        : element === "name" ? appearance.nameColor
        : element === "otherText" ? appearance.otherTextColor
        : appearance.ownTextColor

    function ambienceFor(key) {
        if (key === "otherBubble" || key === "ownBubble") {
            return Theme.highlightBackgroundColor
        }
        return key === "name" ? Theme.highlightColor : Theme.primaryColor
    }

    function writeFor(key, colour) {
        if (key === "otherBubble") {
            appearance.otherBubbleColor = colour
        } else if (key === "ownBubble") {
            appearance.ownBubbleColor = colour
        } else if (key === "name") {
            appearance.nameColor = colour
        } else if (key === "otherText") {
            appearance.otherTextColor = colour
        } else {
            appearance.ownTextColor = colour
        }
    }

    /// Points the spectrum at the selected element's current colour.
    function syncField() {
        colorField.setColor(storedValue !== "" ? storedValue
                                               : String(ambienceFor(element)))
        // The slider stops following its binding once it has been dragged, so
        // both a change of element and a reset have to place it by hand, or
        // it keeps showing a position the bubble no longer has.
        if (opacitySlider) {
            opacitySlider.value = element === "ownBubble" ? appearance.ownBubbleOpacity
                                                          : appearance.otherBubbleOpacity
        }
    }

    // The same fallbacks RoomPage applies, in one place for the preview.
    readonly property color otherBubbleFill:
        Theme.rgba(appearance.otherBubbleColor.length > 0
                   ? appearance.otherBubbleColor : Theme.highlightBackgroundColor,
                   appearance.otherBubbleOpacity)
    readonly property color ownBubbleFill:
        Theme.rgba(appearance.ownBubbleColor.length > 0
                   ? appearance.ownBubbleColor : Theme.highlightBackgroundColor,
                   appearance.ownBubbleOpacity)
    readonly property color nameInk: appearance.nameColor.length > 0
                                     ? appearance.nameColor : Theme.highlightColor
    readonly property color otherInk: appearance.otherTextColor.length > 0
                                      ? appearance.otherTextColor : Theme.primaryColor
    readonly property color ownInk: appearance.ownTextColor.length > 0
                                    ? appearance.ownTextColor : Theme.primaryColor

    Component.onCompleted: syncField()

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        PullDownMenu {
            MenuItem {
                text: qsTr("Reset to defaults")
                onClicked: page.resetAll()
            }
        }

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingSmall

            PageHeader {
                title: qsTr("Appearance")
            }

            // ---- preview ------------------------------------------------

            Rectangle {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                height: previewColumn.height + 2 * Theme.paddingMedium
                radius: Theme.paddingSmall
                color: page.otherBubbleFill

                Column {
                    id: previewColumn

                    anchors {
                        left: parent.left
                        top: parent.top
                        margins: Theme.paddingMedium
                    }
                    spacing: Theme.paddingSmall / 2

                    Label {
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: page.nameInk
                        text: qsTr("Somebody", "sample sender on the appearance page")
                    }

                    Label {
                        font.pixelSize: Theme.fontSizeSmall
                        color: page.otherInk
                        text: qsTr("A received message looks like this.")
                    }
                }
            }

            Rectangle {
                anchors.right: parent.right
                anchors.rightMargin: Theme.horizontalPageMargin
                width: ownPreview.implicitWidth + 2 * Theme.paddingMedium
                height: ownPreview.height + 2 * Theme.paddingMedium
                radius: Theme.paddingSmall
                color: page.ownBubbleFill

                Label {
                    id: ownPreview

                    anchors.centerIn: parent
                    font.pixelSize: Theme.fontSizeSmall
                    color: page.ownInk
                    text: qsTr("And one of my own like this.")
                }
            }

            // ---- what the spectrum colours ------------------------------

            ComboBox {
                label: qsTr("Colouring")
                currentIndex: page.elementKeys.indexOf(page.element)

                menu: ContextMenu {
                    MenuItem { text: qsTr("Their bubble") }
                    MenuItem { text: qsTr("My bubble") }
                    MenuItem { text: qsTr("Sender name") }
                    MenuItem { text: qsTr("Their text") }
                    MenuItem { text: qsTr("My text") }
                }

                onCurrentIndexChanged: {
                    // The guard covers creation order: this fires once while
                    // the spectrum below does not exist yet.
                    if (currentIndex >= 0 && colorField) {
                        page.element = page.elementKeys[currentIndex]
                        page.syncField()
                    }
                }
            }

            TextSwitch {
                text: qsTr("Follow the ambience")
                description: qsTr("Off, the colour below applies")
                checked: page.storedValue === ""
                automaticCheck: false
                onClicked: {
                    if (page.storedValue === "") {
                        // Pins what the spectrum shows right now.
                        page.writeFor(page.element, colorField.chosen)
                    } else {
                        page.writeFor(page.element, "")
                        page.syncField()
                    }
                }
            }

            ColorField {
                id: colorField

                onEdited: page.writeFor(page.element, colour)
            }

            Slider {
                id: opacitySlider

                width: parent.width
                visible: page.element === "otherBubble" || page.element === "ownBubble"
                label: qsTr("Bubble opacity")
                minimumValue: 0.05
                maximumValue: 1.0
                value: page.element === "ownBubble" ? appearance.ownBubbleOpacity
                                                    : appearance.otherBubbleOpacity
                onSliderValueChanged: {
                    if (page.element === "ownBubble") {
                        appearance.ownBubbleOpacity = sliderValue
                    } else {
                        appearance.otherBubbleOpacity = sliderValue
                    }
                }
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }

            // The way back, in plain sight: every colour to the ambience, both
            // opacities to their defaults. The pull-down carries the same
            // entry, but a reset that has to be found is not a safety net.
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Reset to defaults")
                onClicked: page.resetAll()
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }

        VerticalScrollDecorator { }
    }

    function resetAll() {
        appearance.resetAll()
        syncField()
    }
}
