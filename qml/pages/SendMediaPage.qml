import QtQuick 2.0
import Sailfish.Silica 1.0

// What stands between picking a file and sending it: caption, reply and
// second thoughts - none of the three can be added afterwards.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    /// What the picker handed over — may or may not carry a file:// prefix.
    property string path: ""
    property string mimeType: ""
    /// Event this attachment answers, empty for a plain send.
    property string replyTo: ""
    /// Who wrote that event and what it said, for the quote above the caption.
    property string replySender: ""
    property string replyBody: ""

    /// What was already typed when the file was picked, as the caption's start:
    /// text and picture were meant as one message.
    property string caption: ""

    /// Called once the attachment is on its way, so the room can drop the
    /// reply it was holding without losing a half-typed message.
    property var afterSend: null

    /// Called when the send was called off, with the caption as it stands, so
    /// the conversation can put the text back where it was typed.
    property var afterCancel: null

    readonly property bool isImage: mimeType.indexOf("image/") === 0
    // The picker speaks URLs, the core wants a path, and the preview wants a
    // URL again. Normalised once here instead of at each of the three uses.
    readonly property string plainPath: path.indexOf("file://") === 0
                                        ? path.substring(7) : path
    readonly property string fileName: {
        var parts = plainPath.split("/")
        return parts.length > 0 ? parts[parts.length - 1] : plainPath
    }

    canAccept: path.length > 0

    onRejected: {
        if (dialog.afterCancel) {
            dialog.afterCancel(captionField.text)
        }
    }

    /// Who actually sends. Set by the room, which owns the unverified-recipient
    /// warning - a closing dialog cannot put another one over itself.
    property var send: null

    onAccepted: {
        if (dialog.send) {
            // Handed over, not sent: what follows a send belongs after it. Calling
            // `afterSend` here cleared the reply before anything was established.
            dialog.send(dialog.plainPath, dialog.mimeType,
                        captionField.text, dialog.replyTo)
            return
        }
        matrix.sendMedia(dialog.plainPath, dialog.mimeType,
                         captionField.text, dialog.replyTo)
        if (dialog.afterSend) {
            dialog.afterSend()
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            DialogHeader {
                acceptText: dialog.replyTo.length > 0 ? qsTr("Reply") : qsTr("Send")
            }

            // The quote, only when this send answers something. Same shape as in the
            // conversation, so it reads as the same thing.
            Row {
                visible: dialog.replyTo.length > 0
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                spacing: Theme.paddingSmall

                Rectangle {
                    width: Theme.paddingSmall / 2
                    height: quoteTexts.height
                    radius: width / 2
                    color: Theme.rgba(Theme.highlightColor, 0.6)
                }

                Column {
                    id: quoteTexts

                    width: parent.width - Theme.paddingSmall * 1.5

                    Label {
                        width: parent.width
                        visible: text.length > 0
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.highlightColor
                        truncationMode: TruncationMode.Fade
                        textFormat: Text.PlainText
                        text: dialog.replySender
                    }

                    Label {
                        width: parent.width
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.secondaryColor
                        maximumLineCount: 2
                        wrapMode: Text.Wrap
                        elide: Text.ElideRight
                        textFormat: Text.PlainText
                        text: dialog.replyBody
                    }
                }
            }

            // A picture is shown, anything else is named. Guessing at a
            // preview for a PDF would only produce an empty grey box.
            Image {
                visible: dialog.isImage && status === Image.Ready
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                fillMode: Image.PreserveAspectFit
                // Bounded, or a camera picture takes the whole page and the
                // caption field ends up below the fold.
                height: Math.min(implicitHeight, dialog.height / 2)
                asynchronous: true
                source: dialog.isImage ? "file://" + dialog.plainPath : ""
            }

            Item {
                visible: !dialog.isImage
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                height: fileRow.height

                Row {
                    id: fileRow

                    width: parent.width
                    spacing: Theme.paddingMedium

                    Image {
                        anchors.verticalCenter: parent.verticalCenter
                        source: "image://theme/icon-m-file-other"
                    }

                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        width: parent.width - Theme.iconSizeMedium - Theme.paddingMedium
                        wrapMode: Text.WrapAnywhere
                        maximumLineCount: 2
                        elide: Text.ElideRight
                        textFormat: Text.PlainText
                        text: dialog.fileName
                    }
                }
            }

            TextArea {
                id: captionField

                width: parent.width
                label: qsTr("Caption")
                placeholderText: qsTr("Caption (optional)")
                focus: true
                text: dialog.caption
            }
        }
    }
}
