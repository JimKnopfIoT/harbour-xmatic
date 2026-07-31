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
    }

    Column {
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.horizontalPageMargin
        spacing: Theme.paddingLarge

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
                case "ringing": return qsTr("Incoming call")
                case "connecting": return qsTr("Connecting…")
                case "active": return qsTr("Connected")
                default: return matrix.calls.status
                }
            }
        }

        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Theme.paddingLarge

            Button {
                text: qsTr("Accept")
                visible: matrix.calls.state === "ringing"
                onClicked: matrix.calls.acceptCall()
            }

            Button {
                text: matrix.calls.state === "ringing" ? qsTr("Decline") : qsTr("Hang up")
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
}
