import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Share 1.0

// Full size view of an attachment.
//
// The timeline shows a thumbnail; this page asks for the original, which in an
// encrypted room means downloading and decrypting it first. Until it arrives
// the thumbnail already on disk is shown, so the picture never blinks away.
Page {
    id: page

    /// Cache key of the full size image; the thumbnail uses a different one.
    property string mediaKey
    property string source: ""
    /// Name the picture gets when saved, taken from the message.
    property string fileName: ""
    property string mimeType: "image/jpeg"

    property string savedTo: ""

    allowedOrientations: Orientation.All

    Connections {
        target: matrix
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
        contentWidth: Math.max(width, picture.width * picture.scale)
        contentHeight: Math.max(height, picture.height * picture.scale)

        PinchArea {
            width: Math.max(flickable.contentWidth, flickable.width)
            height: Math.max(flickable.contentHeight, flickable.height)

            pinch.target: picture
            pinch.minimumScale: 1.0
            pinch.maximumScale: 6.0
            pinch.dragAxis: Pinch.XAndYAxis

            Image {
                id: picture

                anchors.centerIn: parent
                width: flickable.width
                height: flickable.height
                fillMode: Image.PreserveAspectFit
                asynchronous: true
                smooth: true
                transformOrigin: Item.Center
                source: page.source
            }

            MouseArea {
                anchors.fill: parent
                // Double tap toggles between fit and a useful magnification —
                // the gesture people try first when pinching is awkward.
                onDoubleClicked: picture.scale = picture.scale > 1.0 ? 1.0 : 2.5
            }
        }
    }

    BusyIndicator {
        anchors.centerIn: parent
        size: BusyIndicatorSize.Large
        running: page.source.length === 0
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
