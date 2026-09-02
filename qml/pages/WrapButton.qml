import QtQuick 2.0
import Sailfish.Silica 1.0

// A Silica button whose label may take more than one line: the platform's own
// is single-line and grows off the page. Only the label is ours.
Button {
    id: button

    /// The label. Set this instead of `text`, which stays empty so Silica's
    /// single-line label draws nothing.
    property alias label: buttonLabel.text

    text: ""

    // Never wider than the page. The parent must have a width of its own - inside
    // a Row it comes from the children, and that loop leaves no button on screen.
    width: Math.min(Theme.buttonWidthLarge,
                    parent.width - 2 * Theme.horizontalPageMargin)

    // What makes the background grow. The padding matches the breathing space a
    // one-line button has around its own label.
    implicitHeight: Math.max(Theme.itemSizeExtraSmall,
                             buttonLabel.height + 2 * Theme.paddingMedium)

    Label {
        id: buttonLabel

        // Anchored to the top, not centred: centring in a height derived from the own
        // height is the two-way binding this project has broken itself on.
        anchors {
            top: parent.top
            topMargin: (button.height - height) / 2
            horizontalCenter: parent.horizontalCenter
        }
        width: parent.width - 2 * Theme.paddingMedium
        wrapMode: Text.Wrap
        horizontalAlignment: Text.AlignHCenter

        // Silica's own values, read from the button so a pressed or disabled
        // state looks exactly as it does everywhere else.
        color: button.down ? button.highlightColor : button.color
        font.pixelSize: Theme.fontSizeMedium
    }
}
