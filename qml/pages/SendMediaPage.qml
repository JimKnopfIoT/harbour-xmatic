import QtQuick 2.5
import Sailfish.Silica 1.0

// What stands between picking a file and sending it: caption, reply and
// second thoughts - none of the three can be added afterwards.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    /// What the picker handed over: one entry per file, `path` and `mimeType`
    /// each. Paths may or may not carry a file:// prefix.
    property var files: []

    /// The first of them, which is what the preview shows and what the caption
    /// and the reply belong to.
    readonly property string path: files.length > 0 ? files[0].path : ""
    readonly property string mimeType: files.length > 0 ? files[0].mimeType : ""
    /// Event this attachment answers, empty for a plain send.
    property string replyTo: ""
    /// Who wrote that event and what it said, for the quote above the caption.
    property string replySender: ""
    property string replyBody: ""

    /// What was already typed when the file was picked, as the caption's start:
    /// text and picture were meant as one message.
    property string caption: ""
    /// The room the caption's mentions are looked up in, and those already
    /// picked in the text that became the caption.
    property string roomId: ""
    property var mentionsPicked: ({})

    /// Called when the send was called off, with the caption and its mentions as
    /// they stand, so the conversation can put the text back where it was typed.
    property var afterCancel: null

    /// Whether the picture goes as it lies. Off, it is re-encoded towards a
    /// size a mobile line carries - and loses its metadata on the way.
    property bool original: false

    readonly property bool isImage: mimeType.indexOf("image/") === 0
    /// Empty for an SVG or a canvas past the decode budget: named, not drawn.
    readonly property string previewSource: isImage ? matrix.localImageSource(plainPath) : ""
    /// Sent as they are, like the bridge does: re-encoding loses every frame
    /// but one, or the point of being a drawing.
    readonly property bool keptAsIs: {
        for (var i = 0; i < files.length; i++) {
            var type = files[i].mimeType || ""
            if (type === "image/gif" || type.indexOf("svg") >= 0) {
                return true
            }
        }
        return false
    }
    // The picker speaks URLs, the core wants a path, and the preview wants a
    // URL again. Normalised once here instead of at each of the three uses.
    readonly property string plainPath: path.indexOf("file://") === 0
                                        ? path.substring(7) : path
    readonly property string fileName: {
        var parts = plainPath.split("/")
        return parts.length > 0 ? parts[parts.length - 1] : plainPath
    }

    // Nothing to hand it to is nothing to accept: there is no second way out.
    canAccept: path.length > 0 && dialog.send !== null

    onRejected: {
        if (dialog.afterCancel) {
            dialog.afterCancel(captionField.text, captionMentions.picked)
        }
    }

    /// Who actually sends, and the only way out: set by the room, which owns the
    /// unverified-recipient warning - a closing dialog cannot put another over itself.
    property var send: null

    /// The files with their prefixes taken off, in the order they were picked.
    function plainFiles() {
        var out = []
        for (var i = 0; i < dialog.files.length; i++) {
            var file = dialog.files[i]
            out.push({
                path: file.path.indexOf("file://") === 0 ? file.path.substring(7)
                                                         : file.path,
                mimeType: file.mimeType
            })
        }
        return out
    }

    // Handed over, not sent: what follows a send belongs after it.
    onAccepted: dialog.send(dialog.plainFiles(), captionField.text, dialog.replyTo,
                            dialog.original, captionMentions.ids())

    /// Open while a press on the picker is under way.
    property alias keepKeyboard: keyboardHold.active

    KeyboardHold {
        id: keyboardHold
    }

    function holdKeyboard() {
        keyboardHold.hold()
    }

    MentionInput {
        id: captionMentions

        field: captionField
        roomId: dialog.roomId
        picked: dialog.mentionsPicked
        onKeepKeyboardRequested: dialog.holdKeyboard()
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
                visible: dialog.previewSource.length > 0 && status === Image.Ready
                // As the re-encode will bake it in.
                autoTransform: true
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                fillMode: Image.PreserveAspectFit
                // Bounded, or a camera picture takes the whole page and the
                // caption field ends up below the fold.
                height: Math.min(implicitHeight, dialog.height / 2)
                asynchronous: true
                sourceSize.width: width
                source: dialog.previewSource
            }

            Item {
                visible: dialog.previewSource.length === 0
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

            // What the page cannot show: the others, and where the caption goes.
            Label {
                visible: dialog.files.length > 1
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("%1 files, sent one after another. The caption goes with the first one.")
                          .arg(dialog.files.length)
            }

            // Only where one picture is at stake: several at full size is what
            // the shrinking is there to prevent.
            TextSwitch {
                visible: dialog.isImage && dialog.files.length === 1 && !dialog.keptAsIs
                text: qsTr("Send at original resolution")
                description: qsTr("Off, the picture is made smaller before it goes out and its metadata - the place it was taken, among them - does not travel with it.")
                checked: dialog.original
                automaticCheck: false
                onClicked: dialog.original = !dialog.original
            }

            // The one case the promise above does not cover, said before the send.
            Label {
                visible: dialog.keptAsIs
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Animations and drawings go as they are, with any metadata in them.")
            }

            MentionPicker {
                roomId: dialog.roomId
                armed: captionMentions.armed
                onPicked: captionMentions.insert(id, insert)
                onKeepKeyboardRequested: dialog.holdKeyboard()
            }

            TextArea {
                id: captionField

                width: parent.width
                label: qsTr("Caption")
                placeholderText: qsTr("Caption (optional)")
                focus: true
                text: dialog.caption
                focusOutBehavior: dialog.keepKeyboard ? FocusBehavior.KeepFocus
                                                      : FocusBehavior.ClearItemFocus

                onTextChanged: captionMentions.changed()
                onCursorPositionChanged: captionMentions.changed()
                onActiveFocusChanged: {
                    if (!activeFocus && !dialog.keepKeyboard) {
                        captionMentions.close()
                    }
                }
            }
        }
    }
}
