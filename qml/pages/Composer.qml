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

    Timer {
        id: keepKeyboardWindow

        interval: 400
        onTriggered: composer.keepKeyboard = false
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

        // Hold to record, release to send. A tap-to-start button invites
        // accidental minute-long recordings.
        IconButton {
            id: recordButton

            anchors {
                right: parent.right
                rightMargin: Theme.horizontalPageMargin
                verticalCenter: parent.verticalCenter
            }
            visible: composer.voice && settings.voiceMessages && !composer.editing
                     && messageField.text.trim().length === 0
                     && !messageField.inputMethodComposing
            icon.source: matrix.recorder.recording
                         ? "image://theme/icon-m-mic?" + Theme.errorColor
                         : "image://theme/icon-m-mic"

            onPressAndHold: matrix.recorder.start()
            onReleased: {
                if (matrix.recorder.recording) {
                    matrix.recorder.stop()
                    composer.recordingStopped()
                }
            }
            onCanceled: matrix.recorder.cancel()
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
