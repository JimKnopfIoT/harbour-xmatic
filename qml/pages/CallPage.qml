import QtQuick 2.0
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

    // Leaving the page while a call runs would strand it invisibly.
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
        visible: matrix.calls.selfVideo.active
        fillMode: VideoOutput.PreserveAspectFit
        source: matrix.calls.selfVideo

        // Turned left to right, and only here: this is the preview. Qt 5.6's
        // VideoOutput has no `mirror`, so it is a transform - the pipeline is untouched.
        transform: Scale {
            origin.x: selfView.width / 2
            xScale: -1
        }
    }

    // While the other side's picture fills the page the controls have something to
    // be out of the way of; in a voice call the middle is where they belong.
    readonly property bool videoMode: matrix.calls.remoteVideo.active

    Column {
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.horizontalPageMargin
        spacing: Theme.paddingLarge
        visible: !page.videoMode

        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeLarge
            truncationMode: TruncationMode.Fade
            visible: !matrix.calls.remoteVideo.active
            textFormat: Text.PlainText
            text: matrix.calls.peer.length > 0 ? matrix.calls.peer : qsTr("Call")
        }

        Label {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.secondaryColor
            text: {
                switch (matrix.calls.state) {
                case "calling": return qsTr("Ringing…")
                case "ringing": return matrix.calls.videoOffered || matrix.calls.videoRefused
                                       ? qsTr("Incoming video call") : qsTr("Incoming call")
                case "connecting": return qsTr("Connecting…")
                case "active": return qsTr("Connected")
                default: return matrix.calls.status
                }
            }
        }

        // A video offer that the privacy setting turns down goes through as a
        // voice call. Saying so beats a picture that never appears.
        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            visible: matrix.calls.state === "ringing" && matrix.calls.videoRefused
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryHighlightColor
            text: qsTr("Video calls are switched off in Privacy; this one is answered as a voice call.")
        }

        // Stacked, not in a row: three actions do not fit side by side, and the one
        // that answers as offered belongs on top.
        Column {
            width: parent.width
            spacing: Theme.paddingMedium

            // The width is given, not measured: a wrapping button carries no label of its
            // own, so all three would collapse to the platform minimum.
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

            WrapButton {
                id: endCall

                anchors.horizontalCenter: parent.horizontalCenter
                width: parent.buttonWidth
                label: matrix.calls.state === "ringing" ? qsTr("Decline") : qsTr("Hang up")
                onClicked: matrix.calls.hangUp()
            }
        }

        IconButton {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: matrix.calls.state === "active"
            icon.source: matrix.calls.muted
                         ? "image://theme/icon-m-mic-mute"
                         : "image://theme/icon-m-mic"
            onClicked: matrix.calls.setMuted(!matrix.calls.muted)
        }
    }

    // The same two actions during a video call, in the lower left over the
    // picture. Half transparent: they are a means, the picture is the point.
    Row {
        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge
        }
        spacing: Theme.paddingLarge
        visible: page.videoMode
        opacity: 0.6

        IconButton {
            visible: matrix.calls.state === "active"
            icon.source: matrix.calls.muted
                         ? "image://theme/icon-m-mic-mute"
                         : "image://theme/icon-m-mic"
            onClicked: matrix.calls.setMuted(!matrix.calls.muted)
        }

        IconButton {
            // The cover-sized icon scaled down: Silica ships no hang-up at icon-m, and a
            // picture made smaller stays sharp where one made larger does not.
            icon.source: "image://theme/icon-cover-hangup"
            icon.width: Theme.iconSizeMedium
            icon.height: Theme.iconSizeMedium
            onClicked: matrix.calls.hangUp()
        }
    }
}
