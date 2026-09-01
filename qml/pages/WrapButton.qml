import QtQuick 2.0
import Sailfish.Silica 1.0

// A Silica button whose label may take more than one line.
//
// Silica's own Button cannot: its height is `Theme.itemSizeExtraSmall`, its
// label is a single line with `truncationMode: Fade`, and without a width it
// grows with the text until it leaves the page. In one language that made the
// security page's action run off both edges; in another the same action came
// back faded in the middle of a word. Measured across all 49 button labels in
// the app against all 30 languages: nearly every language has several past
// what fits, and most of them are older than the page that made it visible.
//
// A rule saying "keep labels short" would be a paragraph where a mechanism is
// needed — and some of these actions cannot be said in twenty-six characters
// in Finnish or Hungarian, whatever the rule says.
//
// Nothing of the platform's appearance is reproduced here. `Button`'s
// background is anchored to fill the button minus `(height - implicitHeight)/2`
// top and bottom, so raising `implicitHeight` from outside makes that margin
// zero and the platform's own rectangle grows with the button. Only the label
// is ours, and it takes the colour and the size Silica gives its own.
Button {
    id: button

    /// The label. Set this instead of `text`, which stays empty so Silica's
    /// single-line label draws nothing.
    property alias label: buttonLabel.text

    text: ""

    // Left to the caller, as with any Silica button on a page — but never
    // wider than the page it stands on.
    //
    // The parent must therefore have a width of its own. Inside a `Row`, or a
    // `Column` that was given no width, the parent's width comes from its
    // children instead: that is a binding loop, Qt breaks it by making
    // something zero, and the button then does not exist on screen. The
    // verification page's two answers were invisible for exactly this reason
    // in 0.26.2 and 0.27.0, which is every release this component has shipped
    // in — and nothing in the build says a word about it.
    width: Math.min(Theme.buttonWidthLarge,
                    parent.width - 2 * Theme.horizontalPageMargin)

    // What makes the background grow. The padding matches the breathing space a
    // one-line button has around its own label.
    implicitHeight: Math.max(Theme.itemSizeExtraSmall,
                             buttonLabel.height + 2 * Theme.paddingMedium)

    Label {
        id: buttonLabel

        // Anchored to the top rather than centred: a label that is centred in a
        // height derived from its own height is the kind of two-way binding
        // this project has broken itself on three times. The height above
        // already keeps it in the middle.
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
