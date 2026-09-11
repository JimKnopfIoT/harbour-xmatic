import QtQuick 2.0
import Sailfish.Silica 1.0

// The line a message is written on, shared by the room and the thread. Both had
// their own copy of it, and the four functions that keep the keyboard alive had
// to be repaired in both when the input method turned out to be the culprit.
Column {
    id: composer

    /// What is being written.
    property alias text: messageField.text
    property alias cursorPosition: messageField.cursorPosition
    /// The word the input method has not committed yet - Qt 5.6 offers no
    /// `preeditText`, so this is how a half-typed word is noticed at all.
    readonly property alias composing: messageField.inputMethodComposing

    /// Which offers this composer makes. A thread has neither a paper clip nor
    /// a microphone: it is a text answer.
    property bool attachments: false
    property bool emoji: false
    property bool voice: false
    /// An edit replaces a message; there is nothing to attach to it.
    property bool editing: false

    property string placeholderText: ""

    /// The room whose members can be mentioned. Empty leaves the picker shut.
    property string roomId: ""
    /// What was picked, as name → user id. The text is what the reader sees;
    /// this is what the ping is addressed to.
    property var mentionsPicked: ({})
    /// Whether a mention is being typed. The picker's own condition; see there
    /// why the field's focus cannot be it.
    property bool mentionArmed: false

    signal submitted()
    signal attachRequested()
    signal emojiRequested()
    /// A recording is on its way: the conversation should follow its tail again.
    signal recordingStopped()

    /// Open while an action outside the field is under way. Silica clears a
    /// field's focus on every press outside it - the arrow, the clip and the
    /// format bar are all outside it - and the answer has to still be true one
    /// tick after the press, which is when it asks.
    property bool keepKeyboard: false

    // Hands-free also ends by itself; the conversation follows as after a tap.
    Connections {
        target: matrix.recorder
        onAutoStopped: composer.recordingStopped()
    }

    // A hands-free take never runs on behind the user's back: dropped, not sent.
    Connections {
        target: Qt.application
        onActiveChanged: {
            if (!Qt.application.active && matrix.recorder.handsFree) {
                matrix.recorder.cancel()
            }
        }
    }

    function holdKeyboard() {
        composer.keepKeyboard = true
        keepKeyboardWindow.restart()
    }

    function focusField() {
        messageField.forceActiveFocus()
    }

    function releaseField() {
        messageField.focus = false
    }

    /// Empties the field without assigning `text`. That assignment goes through
    /// Silica's preedit object, which the input method is holding - and the
    /// keyboard folds away with it. The editor's own edits do not.
    function clearField() {
        if (messageField._editor) {
            messageField._editor.remove(0, messageField._editor.length)
        } else {
            messageField.text = ""
        }
    }

    // What the return key left behind on its way to sending. Checked rather than
    // assumed: a keyboard that inserts nothing would otherwise lose a character.
    function dropLineBreak() {
        Qt.inputMethod.commit()
        var position = messageField.cursorPosition
        if (position > 0 && messageField.text.charAt(position - 1) === "\n") {
            if (messageField._editor) {
                messageField._editor.remove(position - 1, position)
            } else {
                messageField.text = messageField.text.slice(0, position - 1)
                        + messageField.text.slice(position)
                messageField.cursorPosition = position - 1
            }
        }
    }

    // A break where no key can make one: the touch keyboard's return carries no
    // shift. The keyboard survives it whatever the setting says - nobody asks
    // for a second line in order to stop typing.
    function insertLineBreak() {
        composer.holdKeyboard()
        Qt.inputMethod.commit()
        var position = messageField.cursorPosition
        if (messageField._editor) {
            messageField._editor.insert(position, "\n")
        } else {
            messageField.text = messageField.text.slice(0, position) + "\n"
                    + messageField.text.slice(position)
            messageField.cursorPosition = position + 1
        }
        composer.focusField()
    }

    /// Puts a character where the cursor stands - an emoji belongs mid-sentence
    /// as often as at the end.
    function insertAtCursor(characters) {
        var position = messageField.cursorPosition
        if (messageField._editor) {
            messageField._editor.insert(position, characters)
        } else {
            messageField.text = messageField.text.slice(0, position) + characters
                    + messageField.text.slice(position)
            messageField.cursorPosition = position + characters.length
        }
        composer.focusField()
    }

    // --- Mentions -----------------------------------------------------------
    // Everything below works on the word the cursor stands in. The input method
    // holds the word being typed until it commits, so the picker follows a beat
    // behind on a predictive keyboard - it cannot lead it.

    /// Where the word around the cursor begins and ends.
    function wordBounds() {
        var text = messageField.text
        var cursor = messageField.cursorPosition
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

    /// Asks for candidates while the word is a mention, and closes the list as
    /// soon as it is not. A name may hold spaces; what is typed may not.
    function refreshMentions() {
        if (composer.roomId.length === 0) {
            return
        }
        var word = composer.wordBounds().word
        if (word.charAt(0) === "@") {
            composer.mentionArmed = true
            matrix.mentions.search(composer.roomId, word.substring(1))
        } else {
            composer.closeMentions()
        }
    }

    /// Shuts the picker without forgetting what was already picked.
    function closeMentions() {
        composer.mentionArmed = false
        matrix.mentions.clear()
    }

    /// Puts the chosen name in place of what was typed. Through the editor, not
    /// through `text`: an assignment folds the keyboard away.
    function insertMention(userId, displayName) {
        composer.holdKeyboard()
        Qt.inputMethod.commit()
        var name = displayName.length > 0 ? displayName : userId
        var label = userId === "@room" ? "@room" : "@" + name
        var bounds = composer.wordBounds()
        // Only a half-typed mention is replaced; from the member page the name
        // is inserted where the cursor stands.
        var from = bounds.word.charAt(0) === "@" ? bounds.start : messageField.cursorPosition
        var to = bounds.word.charAt(0) === "@" ? bounds.end : messageField.cursorPosition
        if (messageField._editor) {
            if (to > from) {
                messageField._editor.remove(from, to)
            }
            messageField._editor.insert(from, label + " ")
        } else {
            messageField.text = messageField.text.slice(0, from) + label + " "
                    + messageField.text.slice(to)
            messageField.cursorPosition = from + label.length + 1
        }
        var picked = composer.mentionsPicked
        picked[label] = userId
        composer.mentionsPicked = picked
        composer.closeMentions()
        composer.focusField()
    }

    /// The mentions a send carries: those whose text is still in the field. A
    /// name that was written over is not addressed any more.
    function mentionIds() {
        var ids = []
        var text = messageField.text
        for (var label in composer.mentionsPicked) {
            if (text.indexOf(label) >= 0 && ids.indexOf(composer.mentionsPicked[label]) < 0) {
                ids.push(composer.mentionsPicked[label])
            }
        }
        return ids
    }

    /// Forgotten with the message they belonged to.
    function clearMentions() {
        composer.mentionsPicked = ({})
        composer.closeMentions()
    }

    Timer {
        id: keepKeyboardWindow

        interval: 400
        onTriggered: composer.keepKeyboard = false
    }

    // Not on every keystroke: a word typed at speed asks once.
    Timer {
        id: mentionWatch

        interval: 120
        onTriggered: composer.refreshMentions()
    }

    MentionPicker {
        roomId: composer.roomId
        armed: composer.mentionArmed
        onPicked: composer.insertMention(userId, displayName)
        onKeepKeyboardRequested: composer.holdKeyboard()
    }

    // Above the field, not over it: a menu on top of the text would cover
    // exactly what was marked.
    FormatBar {
        field: messageField
        onKeepKeyboardRequested: composer.holdKeyboard()
    }

    Item {
        width: parent.width
        height: Math.max(Theme.itemSizeMedium, messageField.height + Theme.paddingMedium)

        // The face keeps the left edge; the platform's messengers put their actions
        // right and start the field at the margin. The icon is drawn, so no IconButton.
        MouseArea {
            id: emojiButton

            anchors {
                left: parent.left
                leftMargin: Theme.paddingMedium
                verticalCenter: parent.verticalCenter
            }
            width: composer.emoji ? Theme.itemSizeSmall : Theme.paddingMedium
            height: Theme.itemSizeSmall
            enabled: composer.emoji

            onClicked: composer.emojiRequested()

            FaceIcon {
                anchors.centerIn: parent
                visible: composer.emoji
                // Measured against the theme's icons, not set to their nominal size: at
                // iconSizeMedium the face came out a third taller than the theme's arrow.
                size: Math.round(Theme.iconSizeMedium * 0.78)
                color: emojiButton.pressed ? Theme.highlightColor : Theme.primaryColor
            }
        }

        TextArea {
            id: messageField

            anchors {
                // Between the two ends: the face left, clip and arrow right.
                left: emojiButton.right
                right: attachButton.visible ? attachButton.left : sendButton.left
                verticalCenter: parent.verticalCenter
            }
            // The field brings a page margin of its own, which in a row with a button at
            // each end is margin twice over. The row's spacing does that job.
            textMargin: 0
            placeholderText: composer.placeholderText
            labelVisible: false

            // Grow with the text, but stop before the composer eats the screen: past the
            // cap the field scrolls internally instead of pushing lines behind the header.
            height: Math.min(implicitHeight, Theme.itemSizeMedium * 3)

            // Not declining to clear the focus - the field has to say so, or the
            // keyboard goes whatever the page does.
            focusOutBehavior: composer.keepKeyboard ? FocusBehavior.KeepFocus
                                                    : FocusBehavior.ClearItemFocus

            // Both, because moving the cursor into a mention is as good a reason
            // to open the list as typing one.
            onTextChanged: mentionWatch.restart()
            onCursorPositionChanged: mentionWatch.restart()
            // Not while the keyboard is being held: that is the press on the
            // picker itself, and it must not shut what it is aiming at.
            onActiveFocusChanged: {
                if (!activeFocus && !composer.keepKeyboard) {
                    composer.closeMentions()
                }
            }

            // The touch keyboard has no modifier on its return key: it always
            // triggers the action, and the break is in the field before this fires.
            EnterKey.enabled: !settings.sendByEnter
                              || messageField.text.trim().length > 0
                              || messageField.inputMethodComposing
            EnterKey.iconSource: settings.sendByEnter ? "image://theme/icon-m-send"
                                                      : "image://theme/icon-m-enter"
            EnterKey.onClicked: {
                if (settings.sendByEnter) {
                    composer.dropLineBreak()
                    composer.submitted()
                }
            }
        }

        // Hold to record, release to send - or tap for hands-free, where a second tap
        // or silence after speech sends. Silence and a ceiling bound a forgotten one.
        IconButton {
            id: recordButton

            /// This press started a held recording: its release sends, and no tap follows.
            property bool held: false

            anchors {
                right: parent.right
                rightMargin: Theme.horizontalPageMargin
                verticalCenter: parent.verticalCenter
            }
            // Stays while recording: it is the way to end a hands-free take.
            visible: matrix.recorder.recording
                     || (composer.voice && settings.voiceMessages && !composer.editing
                         && messageField.text.trim().length === 0
                         && !messageField.inputMethodComposing)
            icon.source: matrix.recorder.recording
                         ? "image://theme/icon-m-mic?" + Theme.errorColor
                         : "image://theme/icon-m-mic"

            onPressed: held = false
            onPressAndHold: {
                if (!matrix.recorder.recording) {
                    held = true
                    matrix.recorder.start()
                }
            }
            onReleased: {
                if (held && matrix.recorder.recording) {
                    matrix.recorder.stop()
                    composer.recordingStopped()
                }
            }
            onCanceled: {
                if (held) {
                    matrix.recorder.cancel()
                }
            }
            onClicked: {
                if (held) {
                    return
                }
                if (matrix.recorder.recording) {
                    matrix.recorder.stop()
                    composer.recordingStopped()
                } else {
                    matrix.recorder.startHandsFree()
                }
            }
        }

        IconButton {
            id: attachButton

            anchors {
                // Held against the arrow even while the microphone stands in its place:
                // both own that slot, never both at once, so the clip does not move.
                right: sendButton.left
                rightMargin: Theme.paddingSmall
                verticalCenter: parent.verticalCenter
            }
            visible: composer.attachments && !composer.editing
            icon.source: "image://theme/icon-m-attach"
            // The same step down as the face at the other end. Both are offers; the send
            // arrow is the action and brightens by itself once there is something to send.
            icon.opacity: Theme.opacityLow
            onClicked: composer.attachRequested()
        }

        IconButton {
            id: sendButton

            anchors {
                right: parent.right
                rightMargin: Theme.horizontalPageMargin
                verticalCenter: parent.verticalCenter
            }
            visible: !recordButton.visible
            enabled: messageField.text.trim().length > 0
                     || messageField.inputMethodComposing
            icon.source: "image://theme/icon-m-send"
            // The arrow fills barely half its box and reads as a smaller button. Scaled
            // rather than replaced: the artwork is the platform's.
            icon.scale: 1.25
            onClicked: composer.submitted()
            // With the return key sending, this is the only way to a line break.
            onPressAndHold: {
                if (settings.sendByEnter) {
                    composer.insertLineBreak()
                }
            }
        }
    }
}
