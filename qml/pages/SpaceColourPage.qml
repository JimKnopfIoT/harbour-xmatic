import QtQuick 2.0
import Sailfish.Silica 1.0

// The colour a space is marked in on the chat list. Derived from the space
// until somebody picks one, and the way back stands on the page itself.
Page {
    id: page

    property string spaceId: ""
    property string spaceName: ""

    // Re-read on the module's revision: picking a colour changes both of these.
    readonly property var marker: (matrix.spaceMarkers.revision,
                                   matrix.spaceMarkers.spaceMarker(page.spaceId))
    readonly property bool chosen: (matrix.spaceMarkers.revision,
                                    matrix.spaceMarkers.colourChosen(page.spaceId))
    readonly property color fill: (marker && marker.colour) || "transparent"

    allowedOrientations: Orientation.All

    Component.onCompleted: colorField.setColor(matrix.spaceMarkers.spaceColour(page.spaceId))

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        Column {
            id: content

            width: page.width
            spacing: Theme.paddingLarge

            PageHeader {
                title: qsTr("Colour")
                description: page.spaceName
            }

            // The colour is drawn nowhere else, so a switched-off mark makes this
            // whole page look broken rather than inactive.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: !settings.spaceInitials
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Marking the space on a room's picture is switched off under Account, Appearance. Until it is on, this colour is drawn nowhere.")
            }

            // The row as the chat list draws it, so the choice is made against
            // what it will look like rather than against a swatch.
            Item {
                width: parent.width
                height: Theme.itemSizeLarge

                Rectangle {
                    id: preview

                    anchors.centerIn: parent
                    width: Theme.iconSizeLarge
                    height: width
                    radius: width / 2
                    color: Theme.rgba(Theme.highlightBackgroundColor, Theme.opacityFaint)

                    Label {
                        anchors.centerIn: parent
                        font.pixelSize: preview.width / 2.5
                        color: Theme.highlightColor
                        textFormat: Text.PlainText
                        text: "?"
                    }

                    Label {
                        anchors {
                            right: parent.right
                            bottom: parent.bottom
                            bottomMargin: -Theme.paddingSmall
                        }
                        text: (page.marker && page.marker.letter) || ""
                        textFormat: Text.PlainText
                        font.pixelSize: Math.round(preview.width * 0.6)
                        font.bold: true
                        color: Qt.rgba(page.fill.r, page.fill.g, page.fill.b, 0.75)
                        style: Text.Outline
                        styleColor: (page.marker && page.marker.outline) || "transparent"
                    }
                }
            }

            TextSwitch {
                text: qsTr("Automatic colour")
                description: qsTr("Taken from the space itself, a different one for each")
                checked: !page.chosen
                automaticCheck: false
                onClicked: {
                    if (page.chosen) {
                        matrix.spaceMarkers.resetSpaceColour(page.spaceId)
                        colorField.setColor(matrix.spaceMarkers.spaceColour(page.spaceId))
                    } else {
                        matrix.spaceMarkers.setSpaceColour(page.spaceId, colorField.chosen)
                    }
                }
            }

            ColorField {
                id: colorField

                onEdited: matrix.spaceMarkers.setSpaceColour(page.spaceId, colour)
            }
        }

        VerticalScrollDecorator { }
    }
}
