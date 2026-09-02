import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Share 1.0

// Full size view of an attachment. Until the original arrives the thumbnail
// already on disk is shown, so the picture never blinks away.
Page {
    id: page

    /// Cache key of the full size image; the thumbnail uses a different one.
    property string mediaKey
    property string source: ""
    /// The fetch failed - refused, dropped, given up on. Without it the indicator
    /// turns for the life of the page over something that is not coming.
    property bool mediaFailed: false
    /// Name the picture gets when saved, taken from the message.
    property string fileName: ""
    property string mimeType: "image/jpeg"

    property string savedTo: ""

    allowedOrientations: Orientation.All

    Connections {
        target: matrix
        onMediaFailed: {
            if (key === page.mediaKey) {
                page.mediaFailed = true
            }
        }

        onMediaReady: {
            if (key === page.mediaKey) {
                page.source = "file://" + path
            }
        }
    }

    SilicaFlickable {
        id: flickable

        anchors.fill: parent
        clip: true

        PullDownMenu {
            MenuItem {
                text: qsTr("Forward")
                enabled: page.source.length > 0
                onClicked: pageStack.push(Qt.resolvedUrl("ForwardPage.qml"), {
                                              path: page.source,
                                              mimeType: page.mimeType
                                          })
            }

            MenuItem {
                text: qsTr("Share")
                enabled: page.source.length > 0
                onClicked: {
                    shareAction.resources = [page.source]
                    shareAction.trigger()
                }
            }

            MenuItem {
                text: qsTr("Save to gallery")
                enabled: page.source.length > 0
                onClicked: {
                    var saved = matrix.saveToPictures(page.source, page.fileName)
                    page.savedTo = saved
                    savedBanner.restart()
                }
            }
        }
        // A rotation changes what "fitted" means, and the sizes are no longer bound
        // after the first zoom, so the zoom starts over.
        onWidthChanged: fitContent()
        onHeightChanged: fitContent()

        function fitContent() {
            contentWidth = width
            contentHeight = height
            returnToBounds()
        }

        // Set once and then owned by `resizeContent()`: zooming rewrites these, which
        // is what makes the picture grow around the fingers.
        contentWidth: width
        contentHeight: height

        // Zoom that stays under the fingers: scaling the picture grows it around its
        // middle, so the pinched spot wanders off.
        PinchArea {
            id: zoom

            width: Math.max(flickable.contentWidth, flickable.width)
            height: Math.max(flickable.contentHeight, flickable.height)

            /// How far in the picture may go, as a multiple of the fitted size.
            readonly property real maximumZoom: 6.0

            property real startWidth: 0
            property real startHeight: 0

            onPinchStarted: {
                startWidth = flickable.contentWidth
                startHeight = flickable.contentHeight
            }

            onPinchUpdated: {
                flickable.contentX += pinch.previousCenter.x - pinch.center.x
                flickable.contentY += pinch.previousCenter.y - pinch.center.y

                var wanted = startWidth * pinch.scale
                var least = flickable.width
                var most = flickable.width * maximumZoom
                var target = Math.max(least, Math.min(wanted, most))
                flickable.resizeContent(target, startHeight * (target / startWidth),
                                        pinch.center)
            }

            onPinchFinished: flickable.returnToBounds()

            Image {
                id: picture

                // The picture *is* the content: growing the content grows it,
                // and panning is then the flickable's ordinary job.
                width: flickable.contentWidth
                height: flickable.contentHeight
                fillMode: Image.PreserveAspectFit
                asynchronous: true
                // A ceiling on the decode, not on the zoom: the allocation must not be the
                // sender's decision. Three times the view keeps a deep zoom sharp.
                sourceSize.width: Math.min(2048, Math.round(flickable.width * 3))
                // Both axes, for the same reason as in the conversation: a
                // width alone lets the height follow the sender's aspect ratio.
                sourceSize.height: Math.min(2048, Math.round(flickable.height * 3))
                smooth: true
                source: page.source
            }

            MouseArea {
                anchors.fill: parent
                // Double tap toggles fit and magnification - the gesture people try first. It
                // zooms around the tapped point, like the pinch.
                onDoubleClicked: {
                    var zoomedIn = flickable.contentWidth > flickable.width * 1.05
                    var target = zoomedIn ? flickable.width : flickable.width * 2.5
                    flickable.resizeContent(target,
                                            flickable.contentHeight
                                            * (target / flickable.contentWidth),
                                            Qt.point(mouse.x, mouse.y))
                    flickable.returnToBounds()
                }
            }
        }
    }

    BusyIndicator {
        anchors.centerIn: parent
        size: BusyIndicatorSize.Large
        running: page.source.length === 0 && !page.mediaFailed
    }

    // A state with no action and no explanation is a dead end. The attachment is
    // still in the room, so trying again is leaving and coming back.
    Label {
        anchors.centerIn: parent
        width: parent.width - 4 * Theme.horizontalPageMargin
        visible: page.mediaFailed
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        color: Theme.secondaryColor
        textFormat: Text.PlainText
        text: qsTr("This attachment could not be loaded")
    }

    ShareAction {
        id: shareAction

        mimeType: page.mimeType
        title: qsTr("Share picture")
    }

    // A short confirmation instead of a system notification: saving is a small
    // action and does not deserve to leave the app.
    Rectangle {
        id: savedNotice

        anchors {
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge
            horizontalCenter: parent.horizontalCenter
        }
        width: noticeLabel.width + 2 * Theme.paddingLarge
        height: noticeLabel.height + 2 * Theme.paddingMedium
        radius: Theme.paddingMedium
        color: Theme.rgba(Theme.highlightDimmerColor, 0.9)
        opacity: 0

        Label {
            id: noticeLabel

            anchors.centerIn: parent
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.primaryColor
            text: page.savedTo.length > 0 ? qsTr("Saved to gallery") : qsTr("Could not save")
        }

        Behavior on opacity { FadeAnimation { } }
    }

    Timer {
        id: savedBanner

        interval: 2500
        onTriggered: savedNotice.opacity = 0
        onRunningChanged: {
            if (running) {
                savedNotice.opacity = 1
            }
        }
    }
}
