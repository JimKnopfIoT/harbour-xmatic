import QtQuick 2.0
import Sailfish.Silica 1.0

// One emoji, drawn as the conversation draws it: the user's picture where there
// is one, otherwise the character - and on a failed decode the character.
Item {
    id: item

    property string character
    property real size: Theme.iconSizeMedium

    // The revision is read on purpose: emojiSource() is a function, so nothing
    // would tell this binding to ask again after a set was read in or removed.
    readonly property string picture: settings.emojiImages && matrix.emojiRevision >= 0
                                      ? matrix.emojiSource(character)
                                      : ""

    width: size
    height: size

    Image {
        id: emojiImage

        anchors.fill: parent
        visible: item.picture.length > 0 && status !== Image.Error
        source: item.picture
        sourceSize.width: item.size
        sourceSize.height: item.size
        asynchronous: true
    }

    Label {
        anchors.centerIn: parent
        visible: !emojiImage.visible
        font.pixelSize: Math.round(item.size * 0.8)
        textFormat: Text.PlainText
        text: item.character
    }
}
