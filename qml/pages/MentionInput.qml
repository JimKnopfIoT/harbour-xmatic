import QtQuick 2.0
import Sailfish.Silica 1.0

// The mention logic for one text field: what is being typed, what was picked,
// which ids a send carries. Shared by the message line and the caption.
// The input method holds the word being typed until it commits, so the picker
// follows a beat behind on a predictive keyboard - it cannot lead it.
Item {
    id: mentions

    /// The field this works on.
    property Item field: null
    /// The room whose members and neighbours can be mentioned. Empty leaves
    /// the picker shut.
    property string roomId: ""
    /// What was picked, as text → user or room id. The text is what the reader
    /// sees; this is what the ping or link is addressed to.
    property var picked: ({})
    /// Whether a mention is being typed. The picker's own condition; see there
    /// why the field's focus cannot be it.
    property bool armed: false

    /// A press outside the field is coming; the owner holds the keyboard.
    signal keepKeyboardRequested()

    // Takes no place in a positioner; the timer runs regardless.
    visible: false

    /// Called on every edit and cursor move; asks once a word stands still.
    function changed() {
        mentionWatch.restart()
    }

    /// Where the word around the cursor begins and ends.
    function wordBounds() {
        var text = field.text
        var cursor = field.cursorPosition
        var start = 0
        for (var back = cursor - 1; back >= 0; --back) {
            if (/\s/.test(text.charAt(back))) {
                start = back + 1
                break
            }
        }
        var end = text.length
        for (var forward = cursor; forward < text.length; ++forward) {
            if (/\s/.test(text.charAt(forward))) {
                end = forward
                break
            }
        }
        return { "start": start, "end": end, "word": text.substring(start, end) }
    }

    /// Whether the word is a mention being typed: `@` a member, `#` a room.
    function isMentionWord(word) {
        return word.charAt(0) === "@" || word.charAt(0) === "#"
    }

    /// Asks for candidates while the word is a mention, and closes the list as
    /// soon as it is not. A name may hold spaces; what is typed may not.
    function refresh() {
        if (mentions.roomId.length === 0 || !field) {
            return
        }
        var word = mentions.wordBounds().word
        if (mentions.isMentionWord(word)) {
            mentions.armed = true
            // The sigil travels: the core tells members from rooms by it.
            matrix.mentions.search(mentions.roomId, word)
        } else {
            mentions.close()
        }
    }

    /// Shuts the picker without forgetting what was already picked.
    function close() {
        mentions.armed = false
        matrix.mentions.clear()
    }

    /// Puts the chosen text in place of what was typed. Through the editor, not
    /// through `text`: an assignment folds the keyboard away.
    function insert(id, label) {
        mentions.keepKeyboardRequested()
        Qt.inputMethod.commit()
        var bounds = mentions.wordBounds()
        // Only a half-typed mention is replaced; from the member page the name
        // is inserted where the cursor stands.
        var typed = mentions.isMentionWord(bounds.word)
        var from = typed ? bounds.start : field.cursorPosition
        var to = typed ? bounds.end : field.cursorPosition
        if (field._editor) {
            if (to > from) {
                field._editor.remove(from, to)
            }
            field._editor.insert(from, label + " ")
        } else {
            field.text = field.text.slice(0, from) + label + " " + field.text.slice(to)
            field.cursorPosition = from + label.length + 1
        }
        var all = mentions.picked
        all[label] = id
        mentions.picked = all
        mentions.close()
        field.forceActiveFocus()
    }

    /// The mentions a send carries: those whose text is still in the field. A
    /// name that was written over is not addressed any more.
    function ids() {
        var out = []
        var text = field ? field.text : ""
        for (var label in mentions.picked) {
            if (text.indexOf(label) >= 0 && out.indexOf(mentions.picked[label]) < 0) {
                out.push(mentions.picked[label])
            }
        }
        return out
    }

    /// Forgotten with the text they belonged to.
    function clear() {
        mentions.picked = ({})
        mentions.close()
    }

    // Not on every keystroke: a word typed at speed asks once.
    Timer {
        id: mentionWatch

        interval: 120
        onTriggered: mentions.refresh()
    }
}
