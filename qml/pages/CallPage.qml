import QtQuick 2.5
import Sailfish.Silica 1.0
import QtMultimedia 5.6

// The call itself. Deliberately sparse: during a call there is exactly one
// thing to decide — keep talking or stop.
Page {
    id: page

    objectName: "callPage"

    allowedOrientations: Orientation.All
    // Leaving is allowed: the call keeps running and the room's pull-down
    // menu leads back to it.
    backNavigation: true

    // The picture being sent has to follow how the phone is held, or turning
    // the device leaves the other side looking at a sideways image.
    onOrientationChanged: matrix.calls.setOrientation(orientation)
    Component.onCompleted: matrix.calls.setOrientation(orientation)

    // Leaving the page while a call runs would strand it invisibly, so it goes as
    // soon as the call is over - always. Holding it open to show why a call
    // failed was tried and taken back: the page that stays is the page the *next*
    // incoming call finds in its way (`onIncomingCall` only pushes when none is
    // on top), and then a ringing phone cannot be answered at all. The reason is
    // told by a notification instead, which needs nothing to be dismissed.
    Connections {
        target: matrix.calls
        onCallChanged: {
            if (matrix.calls.state === "idle") {
                pageStack.pop()
            }
        }
    }

    // The other side's picture, when the call carries one. It fills the page
    // and the controls float above it.
    VideoOutput {
        anchors.fill: parent
        visible: matrix.calls.remoteVideo.active
        fillMode: VideoOutput.PreserveAspectFit
        source: matrix.calls.remoteVideo
    }

    // The own camera, small and out of the way — the same picture the other
    // side receives, already rotated by the pipeline.
    VideoOutput {
        id: selfView

        anchors {
            right: parent.right
            bottom: parent.bottom
            margins: Theme.paddingLarge
        }
        width: Theme.itemSizeHuge * 1.4
        height: width * 3 / 4
        visible: matrix.calls.selfVideo.active && !matrix.calls.gpuCapture
        fillMode: VideoOutput.PreserveAspectFit
        source: matrix.calls.selfVideo

        // Turned left to right, and only here: this is the preview. Qt 5.6's
        // VideoOutput has no `mirror`, so it is a transform - the pipeline is untouched.
        transform: Scale {
            origin.x: selfView.width / 2
            xScale: -1
        }
    }

    // Where the camera gives nothing to plain memory, the call takes this
    // viewfinder's pictures instead; it is the self-view then.
    Camera {
        id: gpuCamera

        position: Camera.FrontFace
        captureMode: Camera.CaptureStillImage
        cameraState: matrix.calls.gpuCapture ? Camera.ActiveState : Camera.UnloadedState
    }

    VideoOutput {
        id: gpuView

        // Turned a quarter, the item still lays out upright: shift by the overhang.
        readonly property real overhang: rotation % 180 !== 0 ? (height - width) / 2 : 0

        anchors {
            right: parent.right
            bottom: parent.bottom
            rightMargin: Theme.paddingLarge + overhang
            bottomMargin: Theme.paddingLarge - overhang
        }
        // Portrait whatever the page does: the grab is taken in this item's own frame.
        // Turned back against the page, so the preview stays upright.
        width: Theme.itemSizeHuge * 1.05
        height: width * 4 / 3
        rotation: {
            switch (page.orientation) {
            case Orientation.Landscape: return -90
            case Orientation.LandscapeInverted: return 90
            case Orientation.PortraitInverted: return 180
            default: return 0
            }
        }
        visible: matrix.calls.gpuCapture
        fillMode: VideoOutput.PreserveAspectCrop
        source: gpuCamera
    }

    // Fifteen a second, one at a time: a grab still in flight skips the tick.
    Timer {
        id: grabTimer

        property bool grabbing: false

        interval: 66
        repeat: true
        running: matrix.calls.gpuCapture && gpuCamera.cameraStatus === Camera.ActiveStatus
        onTriggered: {
            if (grabbing) {
                return
            }
            grabbing = true
            var started = gpuView.grabToImage(function(result) {
                grabTimer.grabbing = false
                matrix.calls.pushGrabbedFrame(result.image)
            }, Qt.size(480, 640))
            if (!started) {
                grabbing = false
            }
        }
    }

    // The other side's picture hides the name and the status in the middle.
    readonly property bool videoMode: matrix.calls.remoteVideo.active

    // No `visible` on this column: it carries the answer buttons, and a container
    // that hides takes every entry with it.
    Column {
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.horizontalPageMargin
        spacing: Theme.paddingLarge

        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeLarge
            truncationMode: TruncationMode.Fade
            visible: !page.videoMode
            textFormat: Text.PlainText
            text: matrix.calls.peer.length > 0 ? matrix.calls.peer : qsTr("Call")
        }

        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.secondaryColor
            visible: !page.videoMode
            text: {
                switch (matrix.calls.state) {
                case "calling": return qsTr("Ringing…")
                case "ringing": return matrix.calls.videoOffered || matrix.calls.videoRefused
                                       ? qsTr("Incoming video call") : qsTr("Incoming call")
                case "connecting": return qsTr("Connecting…")
                case "active": return qsTr("Connected")
                default: return matrix.calls.failure.length > 0
                                ? matrix.calls.failure
                                : matrix.calls.status
                }
            }
        }

        // A video offer that the privacy setting turns down goes through as a
        // voice call. Saying so beats a picture that never appears.
        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            visible: !page.videoMode && matrix.calls.state === "ringing"
                     && matrix.calls.videoRefused
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryHighlightColor
            text: qsTr("Video calls are switched off in Privacy; this one is answered as a voice call.")
        }

        // Stacked, not in a row: the labels do not fit side by side, and the one
        // that answers as offered belongs on top.
        Column {
            width: parent.width
            spacing: Theme.paddingMedium

            // The width is given, not measured: a wrapping button carries no label of its
            // own, so both would collapse to the platform minimum.
            readonly property real buttonWidth: Math.min(
                    Theme.buttonWidthLarge,
                    width - 2 * Theme.horizontalPageMargin)

            WrapButton {
                id: withCamera

                anchors.horizontalCenter: parent.horizontalCenter
                width: parent.buttonWidth
                label: qsTr("Accept with camera")
                visible: matrix.calls.state === "ringing" && matrix.calls.videoOffered
                onClicked: matrix.calls.acceptCall(true)
            }

            WrapButton {
                id: plainAccept

                anchors.horizontalCenter: parent.horizontalCenter
                width: parent.buttonWidth
                label: matrix.calls.videoOffered ? qsTr("Accept without camera")
                                                : qsTr("Accept")
                visible: matrix.calls.state === "ringing"
                onClicked: matrix.calls.acceptCall(false)
            }
        }
    }

    // The same controls with or without a picture; the picture only moves the
    // name and the status out of the way.
    Row {
        // Lower left: the own picture sits in the lower right.
        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge
        }
        spacing: Theme.paddingLarge
        opacity: page.videoMode ? 0.6 : 1.0

        IconButton {
            visible: matrix.calls.state === "active"
            icon.source: matrix.calls.muted
                         ? "image://theme/icon-m-mic-mute"
                         : "image://theme/icon-m-mic"
            onClicked: matrix.calls.setMuted(!matrix.calls.muted)
        }

        // Hangs up, and declines while ringing.
        IconButton {
            // The cover-sized icon scaled down: Silica ships no hang-up at icon-m.
            icon.source: "image://theme/icon-cover-hangup"
            icon.width: Theme.iconSizeMedium
            icon.height: Theme.iconSizeMedium
            onClicked: matrix.calls.hangUp()
        }
    }
}
