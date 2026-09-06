import QtQuick 2.0
import Sailfish.Silica 1.0

// What the homeserver says a linked page is: host first, then title and
// description, a picture where the server re-hosted one. Width comes from
// the caller; nothing here measures the parent.
Item {
    id: card

    /// The address, exactly as it stands in the message.
    property string url

    /// Set by the caller; the children anchor to this item.
    property real availableWidth: 0

    /// The core's answer, once it is in. Empty until then and empty for a page
    /// that has nothing to say — the card stays collapsed either way.
    property var preview: ({})

    readonly property bool available: !!preview && preview.available === true
    readonly property bool hasImage: available && !!preview.image
    readonly property string mediaKey: "preview:" + url

    signal activated(string link)

    width: availableWidth
    height: available ? frame.height : 0
    visible: available

    function load() {
        if (card.url.length === 0) {
            return
        }
        card.preview = matrix.linkPreviews.preview(card.url)
    }

    Component.onCompleted: load()
    onUrlChanged: load()

    Connections {
        target: matrix.linkPreviews
        onPreviewReady: {
            if (url === card.url) {
                card.preview = preview
            }
        }
        onRetryDue: {
            if (url === card.url && !card.available) {
                card.load()
            }
        }
    }

    Rectangle {
        id: frame

        width: parent.width
        height: Math.max(thumb.visible ? thumb.height : 0, texts.height)
                + 2 * Theme.paddingSmall
        radius: Theme.paddingSmall
        color: Theme.rgba(Theme.primaryColor, 0.06)

        Image {
            id: thumb

            anchors {
                left: parent.left
                top: parent.top
                margins: Theme.paddingSmall
            }
            width: Theme.itemSizeMedium
            height: Theme.itemSizeMedium
            visible: card.hasImage
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            sourceSize.width: Theme.itemSizeMedium
            sourceSize.height: Theme.itemSizeMedium
            source: {
                if (!card.hasImage) {
                    return ""
                }
                var known = matrix.mediaPath(card.mediaKey)
                return known.length > 0 ? "file://" + known : ""
            }

            // Asked when the card shows and when it was already showing at
            // creation: a remembered preview never changes `visible`.
            function fetch() {
                if (visible && source == "") {
                    matrix.requestMedia(card.mediaKey, card.preview.image, true,
                                        card.preview.imageSize || 0)
                }
            }
            onVisibleChanged: fetch()
            Component.onCompleted: fetch()

            Connections {
                target: matrix
                onMediaReady: {
                    if (key === card.mediaKey) {
                        thumb.source = "file://" + path
                    }
                }
            }
        }

        Column {
            id: texts

            anchors {
                left: thumb.visible ? thumb.right : parent.left
                right: parent.right
                top: parent.top
                margins: Theme.paddingSmall
            }
            spacing: Theme.paddingSmall / 4

            // The host before the title: the card says where the link goes,
            // not only what the page claims to be.
            Label {
                width: parent.width
                truncationMode: TruncationMode.Fade
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeExtraSmall
                font.bold: true
                color: Theme.highlightColor
                text: card.available ? (card.preview.host || "") : ""
            }

            Label {
                width: parent.width
                visible: text.length > 0
                wrapMode: Text.Wrap
                maximumLineCount: 2
                truncationMode: TruncationMode.Fade
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.primaryColor
                text: card.available ? (card.preview.title || "") : ""
            }

            Label {
                width: parent.width
                visible: text.length > 0
                wrapMode: Text.Wrap
                maximumLineCount: 2
                truncationMode: TruncationMode.Fade
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: card.available ? (card.preview.description || "") : ""
            }
        }

        MouseArea {
            anchors.fill: parent
            onClicked: card.activated(card.url)
        }
    }
}
