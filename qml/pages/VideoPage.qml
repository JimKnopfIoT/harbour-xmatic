import QtQuick 2.0
import Sailfish.Silica 1.0
import QtMultimedia 5.6

// Plays a received video from the local cache.
//
// The file has to be downloaded — and in an encrypted room decrypted — before
// playback can start, so the page opens on a spinner and begins by itself.
Page {
    id: page

    property string mediaKey
    property string source: ""
    property string fileName: ""

    allowedOrientations: Orientation.All

    Connections {
        target: matrix
        onMediaReady: {
            if (key === page.mediaKey) {
                page.source = "file://" + path
            }
        }
    }

    Video {
        id: player

        anchors.fill: parent
        source: page.source
        autoPlay: true
        fillMode: VideoOutput.PreserveAspectFit

        MouseArea {
            anchors.fill: parent
            onClicked: player.playbackState === MediaPlayer.PlayingState
                       ? player.pause()
                       : player.play()
        }
    }

    // Playback controls stay minimal: tap toggles, the position bar is the
    // only chrome, and it fades with the rest of the page.
    Slider {
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge
        }
        visible: player.duration > 0
        minimumValue: 0
        maximumValue: player.duration
        value: player.position
        onReleased: player.seek(value)
    }

    BusyIndicator {
        anchors.centerIn: parent
        size: BusyIndicatorSize.Large
        running: page.source.length === 0
    }

    PullDownMenu {
        MenuItem {
            text: qsTr("Save")
            enabled: page.source.length > 0
            onClicked: matrix.saveToDownloads(page.source, page.fileName)
        }
    }
}
