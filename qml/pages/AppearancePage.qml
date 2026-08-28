import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0

// The conversation's colours, chosen by the user. One spectrum serves every
// element: the selector underneath decides what the fingertip is colouring,
// the preview at the top uses the very expressions the conversation uses, so
// a choice shows its effect before it ever hits a chat — and the pull-down's
// reset is the way back out of an unreadable palette. Every element defaults
// to "follow the ambience", which reproduces the built-in look.
Page {
    id: page

    allowedOrientations: Orientation.All

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

            // One width for the buttons on this page - see AccountPage.
            readonly property real buttonWidth: Math.min(
                    width - 2 * Theme.horizontalPageMargin,
                    Math.max(resetColoursButton.implicitWidth,
                             choosePicturesButton.implicitWidth,
                             removePicturesButton.implicitWidth))

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
                id: resetColoursButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                text: qsTr("Reset colours to defaults")
                onClicked: page.resetAll()
            }

            TextSwitch {
                text: qsTr("Hide the keyboard after sending")
                description: qsTr("On, the keyboard closes once a message is out and the conversation is back in full. Off, it stays up for the next one.")
                checked: settings.hideKeyboardOnSend
                automaticCheck: false
                onClicked: settings.hideKeyboardOnSend = !settings.hideKeyboardOnSend
            }

            TextSwitch {
                text: qsTr("Reactions as pictures")
                // The path and the warning belong where the choice is made,
                // not in a manual nobody has.
                description: qsTr("Off, a reaction is drawn as the character it is - always right and free. On, xmatic looks for a picture of your own for it in %1, named after its code points (1f44d.svg). Nothing is shipped and nothing is downloaded. Weigh it up: a picture file is opened by an image decoder, which is where an app of this kind is most exposed.").arg(matrix.emojiDirectory)
                checked: settings.emojiImages
                automaticCheck: false
                onClicked: settings.emojiImages = !settings.emojiImages
            }

            // Reading a set in, rather than copying files onto the device by
            // hand. What it buys is the checking: only files named like code
            // points, only small ones, each one decoded once here and written
            // out again as PNG, each one with a checksum that is verified
            // every time the picture is used afterwards.
            Button {
                id: choosePicturesButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                enabled: !emojiSet.busy
                text: qsTr("Choose emoji pictures")
                onClicked: pageStack.push(folderPicker)
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: emojiSet.busy
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Reading the pictures…")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: !emojiSet.busy && (emojiSet.lastImported > 0 || emojiSet.lastRejected > 0)
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("%1 taken over, %2 refused")
                          .arg(emojiSet.lastImported).arg(emojiSet.lastRejected)
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: emojiSet.tampered
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.errorColor
                text: qsTr("The pictures have changed since they were read in and are not shown.")
            }

            Button {
                id: removePicturesButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                visible: emojiSet.verified && !emojiSet.busy
                text: qsTr("Remove emoji pictures")
                onClicked: emojiSet.removeAll()
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }

        VerticalScrollDecorator { }
    }

    // The picker hands back a folder; everything else happens in the core of
    // the app, not here.
    Component {
        id: folderPicker

        FolderPickerPage {
            allowedOrientations: Orientation.All
            onSelectedPathChanged: emojiSet.importFrom(selectedPath)
        }
    }

    function resetAll() {
        appearance.resetAll()
        // Both the colour and the two sliders: the reset is a change from
        // outside, and nothing in this page follows one on its own.
        syncField()
        colorField.placeSliders()
    }
}
