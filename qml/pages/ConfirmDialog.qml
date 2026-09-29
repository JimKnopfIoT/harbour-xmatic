import QtQuick 2.0
import Sailfish.Silica 1.0

// One confirmation before what cannot be taken back, and it names what it acts
// on: a room left by accident could not even be rejoined, nothing had named it.
Dialog {
    id: dialog

    // The question, answered by accepting: "Really leave this room?"
    property string question
    // What the question is about — a room, a space, a person. Shown big.
    property string subject
    // What accepting costs, in one or two sentences. Optional.
    property string explanation
    // The accept label. Says the deed ("Leave"), never "OK".
    property string acceptLabel
    // The platform's "Cancel" instead of "Keep": nothing is kept by not opening a link.
    property bool plainCancel: false
    // Optional content under the text, e.g. the map behind a location link.
    property url extraSource
    property var extraProperties: ({})
    readonly property bool hasExtra: extraSource.toString().length > 0
    /// Landscape with extra content: the subject takes one small line.
    readonly property bool compact: isLandscape && hasExtra

    allowedOrientations: Orientation.All

    // Landscape on a small screen has almost no vertical room, so the content
    // scrolls rather than being cut off.
    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        VerticalScrollDecorator {}

        Column {
            id: column

            width: parent.width
            spacing: Theme.paddingLarge

            DialogHeader {
                acceptText: dialog.acceptLabel
                // Set once, not bound: left alone it keeps the platform's own text.
                Component.onCompleted: {
                    if (!dialog.plainCancel) {
                        cancelText = qsTr("Keep")
                    }
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryColor
                // Under a map the header and the address say it already.
                visible: !dialog.hasExtra
                textFormat: Text.PlainText
                text: dialog.question
            }

            // The whole point of the dialog. A room name can be long and can
            // contain anything a user typed, so it wraps and stays plain text.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: dialog.compact ? Text.NoWrap : Text.Wrap
                truncationMode: dialog.compact ? TruncationMode.Fade : TruncationMode.None
                font.pixelSize: dialog.compact ? Theme.fontSizeSmall : Theme.fontSizeLarge
                color: Theme.highlightColor
                textFormat: Text.PlainText
                text: dialog.subject
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                visible: dialog.explanation.length > 0
                textFormat: Text.PlainText
                text: dialog.explanation
            }

            // The rest of the page; in landscape at least a usable area, and the page scrolls.
            Loader {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                height: active ? Math.max(dialog.height - y - Theme.paddingLarge,
                                          Theme.itemSizeHuge * 2) : 0
                active: dialog.hasExtra
                visible: active
                Component.onCompleted: {
                    if (active) {
                        setSource(dialog.extraSource, dialog.extraProperties)
                    }
                }
            }
        }
    }
}
