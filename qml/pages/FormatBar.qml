import QtQuick 2.0
import Sailfish.Silica 1.0

// The wrappers over the composer, shown while something is selected. The field
// does the selecting itself - Silica's long press marks a word and gives it the
// two handles; this only puts markers around what they hold.
Row {
    id: bar

    /// The field being formatted, set by whoever places the bar.
    property Item field: null

    /// The selection as it was last seen. Remembered rather than read at the
    /// tap: a press outside the field takes its focus, and with it the live
    /// selection - the range would be gone exactly when it is needed.
    property int from: 0
    property int to: 0

    readonly property bool armed: to > from

    /// Emitted before the field is written to: the bar is outside the field, and
    /// Silica clears a field's focus for a press outside it.
    signal keepKeyboardRequested()

    width: parent.width
    height: armed ? Theme.itemSizeSmall : 0
    visible: armed

    function forget() {
        bar.from = 0
        bar.to = 0
    }

    function remember() {
        if (bar.field && bar.field.selectionStart !== bar.field.selectionEnd) {
            bar.from = bar.field.selectionStart
            bar.to = bar.field.selectionEnd
        }
    }

    /// Puts `marker` around the remembered range and hands the field back.
    function wrap(marker) {
        if (!bar.armed || !bar.field) {
            return
        }
        var text = bar.field.text
        var start = Math.min(bar.from, text.length)
        var end = Math.min(bar.to, text.length)
        if (end <= start) {
            bar.forget()
            return
        }
        var after = end + 2 * marker.length
        bar.keepKeyboardRequested()
        bar.field.text = text.slice(0, start) + marker + text.slice(start, end)
                         + marker + text.slice(end)
        bar.forget()
        bar.field.forceActiveFocus()
        bar.field.cursorPosition = after
    }

    Connections {
        target: bar.field

        onSelectionStartChanged: bar.remember()
        onSelectionEndChanged: bar.remember()
        // Typing dismisses the bar, and so does a tap that only moves the
        // cursor: both mean the marked range is no longer what was meant.
        onTextChanged: bar.forget()
        onCursorPositionChanged: {
            if (bar.field && bar.field.selectionStart === bar.field.selectionEnd) {
                bar.forget()
            }
        }
    }

    Repeater {
        // The letter carries its own meaning: no icon exists for these in the
        // theme, and a drawn one would need explaining in every language.
        model: [
            { "marker": "**", "letter": "B", "bold": true },
            { "marker": "*", "letter": "I", "italic": true },
            { "marker": "~~", "letter": "S", "strikeout": true },
            { "marker": "++", "letter": "U", "underline": true },
            { "marker": "`", "letter": "M", "mono": true }
        ]

        MouseArea {
            width: Math.floor(bar.width / 5)
            height: bar.height

            onClicked: bar.wrap(modelData.marker)

            Label {
                anchors.centerIn: parent
                font.pixelSize: Theme.fontSizeLarge
                font.bold: modelData.bold === true
                font.italic: modelData.italic === true
                font.strikeout: modelData.strikeout === true
                font.underline: modelData.underline === true
                font.family: modelData.mono === true ? "monospace" : Theme.fontFamily
                color: parent.pressed ? Theme.highlightColor : Theme.primaryColor
                text: modelData.letter
            }
        }
    }
}
