import QtQuick 2.0

// Open while a press outside a text field is under way. Silica clears a field's
// focus on every press outside it, and the answer has to still be true one tick
// after the press, which is when the field asks. Shared by every field that has
// buttons beside it.
Timer {
    id: keyboardHold

    /// Read by the field's `focusOutBehavior`.
    property bool active: false

    function hold() {
        keyboardHold.active = true
        keyboardHold.restart()
    }

    interval: 400
    onTriggered: keyboardHold.active = false
}
