import QtQuick 2.0
import Sailfish.Silica 1.0
import QtMultimedia 5.6

// Plays a received video from the cache. It has to be downloaded and decrypted
// first, so the page opens on a spinner and begins by itself.
Page {
    id: page

    property string mediaKey
    property string source: ""
    /// The fetch failed - refused, dropped, given up on. Without it the indicator
    /// turns for the life of the page over something that is not coming.
    property bool mediaFailed: false
    property string fileName: ""
    /// The event's own figure, shown while the download runs. Zero where the
    /// sender declared none - and a claim either way, so it is only shown while
    /// it is inside what the core would actually fetch.
    property real declaredSize: 0
    /// Chosen on the still before the page opened, so a loud video never plays
    /// a note. Only the initial state - the button on the page takes over.
    property bool startMuted: false

    readonly property bool loading: page.source.length === 0 && !page.mediaFailed
    /// What `media::fetch` refuses outright. A larger figure is not information
    /// about this video, it is a number that is about to be rejected.
    readonly property bool sizeWorthShowing: page.declaredSize > 0
                                             && page.declaredSize <= 100 * 1024 * 1024

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

    Video {
        id: player

        anchors.fill: parent
        source: page.source
        autoPlay: true
        muted: page.startMuted
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
        id: positionBar

        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge
        }
        visible: player.duration > 0
        minimumValue: 0
        maximumValue: player.duration
        onReleased: player.seek(value)
    }

    // Silica writes `value` imperatively as soon as the bar is touched, and that
    // destroys a plain binding for good: one tap and the bar stood still for the
    // rest of the video. Re-established the moment the finger is off.
    Binding {
        target: positionBar
        property: "value"
        value: player.position
        when: !positionBar.down
    }

    // The wait gets a figure where the event carries one: a spinner alone does not
    // say whether this is a moment or a hundred megabytes.
    Column {
        anchors {
            left: parent.left
            right: parent.right
            verticalCenter: parent.verticalCenter
        }
        spacing: Theme.paddingMedium
        visible: page.loading

        BusyIndicator {
            anchors.horizontalCenter: parent.horizontalCenter
            size: BusyIndicatorSize.Large
            running: page.loading
        }

        Label {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: page.sizeWorthShowing
            color: Theme.secondaryColor
            font.pixelSize: Theme.fontSizeSmall
            text: Format.formatFileSize(page.declaredSize)
        }
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

    // Sound off without leaving the video. Over the bar rather than at the top
    // edge: the pulley is there, and so is the camera notch on some devices.
    IconButton {
        // Anchored to the page, not to the bar: the bar is hidden until a duration
        // is known, and a zero-height anchor put this at the very bottom edge.
        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
            bottom: parent.bottom
            bottomMargin: positionBar.visible
                          ? positionBar.height + 2 * Theme.paddingLarge
                          : Theme.paddingLarge
        }
        // Also before the file is here: `muted` is set on the player ahead of time,
        // so a video that turns out to be loud can be opened silent.
        visible: !page.mediaFailed
        opacity: 0.6
        icon.source: player.muted
                     ? "image://theme/icon-m-speaker-mute"
                     : "image://theme/icon-m-speaker"
        onClicked: player.muted = !player.muted
    }

    PullDownMenu {
        MenuItem {
            text: qsTr("Save")
            enabled: page.source.length > 0
            onClicked: matrix.saveToDownloads(page.source, page.fileName)
        }
    }
}
