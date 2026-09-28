import QtQuick 2.0
import Sailfish.Silica 1.0

// The own position into one room: once, or live for a while. Offered only
// where Privacy allows it; the gate itself sits in matrix.locations.
Page {
    id: page

    allowedOrientations: Orientation.All

    property string roomId
    /// Unknown counts as encrypted, as for every map.
    property bool encrypted: true

    readonly property var position: matrix.locations.position
    readonly property bool hasFix: !!position && typeof position.lat === "number"
    readonly property bool sharingHere: matrix.locations.liveRooms.indexOf(page.roomId) >= 0
    readonly property var durations: [60, 360, 1440]

    onStatusChanged: {
        if (status === PageStatus.Activating) {
            matrix.clearLastError()
            matrix.locations.locate()
        } else if (status === PageStatus.Deactivating) {
            matrix.locations.release()
        }
    }
    Component.onDestruction: matrix.locations.release()

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            readonly property real buttonWidth: Math.min(
                    Theme.buttonWidthLarge,
                    width - 2 * Theme.horizontalPageMargin)

            PageHeader {
                title: qsTr("Share location")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: matrix.lastError.length > 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                textFormat: Text.PlainText
                text: matrix.lastError
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: matrix.locations.locating && !page.hasFix
                visible: running
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: !page.hasFix
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: matrix.locations.positionError.length > 0 ? Theme.errorColor
                                                                  : Theme.secondaryHighlightColor
                textFormat: Text.PlainText
                text: matrix.locations.positionError.length > 0
                      ? matrix.locations.positionError
                      : qsTr("Finding your position…")
            }

            // Where it is still waiting for a fix, and where the device refused one.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                visible: !page.hasFix && !matrix.locations.locating
                label: qsTr("Try again")
                onClicked: matrix.locations.locate()
            }

            LocationBlock {
                x: Theme.horizontalPageMargin
                availableWidth: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.hasFix
                location: page.hasFix ? {
                                            "lat": page.position.lat,
                                            "lon": page.position.lon,
                                            "accuracy": page.position.accuracy,
                                            "live": false
                                        } : null
                encrypted: page.encrypted
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                enabled: page.hasFix
                label: qsTr("Send this position")
                onClicked: {
                    if (matrix.locations.sendCurrent()) {
                        pageStack.pop()
                    }
                }
            }

            SectionHeader {
                text: qsTr("Live location")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                textFormat: Text.PlainText
                text: qsTr("Your position goes to this room when you move, while the app is open, until the time is up or you stop it. When the app is closed, the others see the last position until the end of the time.")
            }

            ComboBox {
                id: durationBox

                width: parent.width
                visible: !page.sharingHere
                label: qsTr("For")
                currentIndex: 0

                menu: ContextMenu {
                    MenuItem { text: qsTr("1 hour") }
                    MenuItem { text: qsTr("6 hours") }
                    MenuItem { text: qsTr("1 day") }
                }
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                visible: !page.sharingHere
                label: qsTr("Share live location")
                onClicked: matrix.locations.startLive(page.roomId,
                                                      page.durations[durationBox.currentIndex])
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.sharingHere
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.highlightColor
                textFormat: Text.PlainText
                text: page.sharingHere
                      ? qsTr("Sharing your live location until %1.")
                        .arg(Format.formatDate(new Date(matrix.locations.liveUntil(page.roomId)),
                                               Formatter.TimeValue))
                      : ""
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                visible: page.sharingHere
                label: qsTr("Stop sharing")
                onClicked: matrix.locations.stopLive(page.roomId)
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }
    }
}
