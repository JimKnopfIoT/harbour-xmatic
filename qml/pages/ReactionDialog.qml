import QtQuick 2.0
import Sailfish.Silica 1.0

// Picking a reaction for one message.
//
// A reaction is not an icon set: Matrix carries it as a string, usually one
// emoji character, and whatever another client sent has to be drawn as it
// arrived. So these are characters, not images - the device's emoji font draws
// them, the same one that draws them in the conversation.
//
// The row below is the common handful. Anything else is typed in the field:
// the system keyboard has every emoji there is, and building a second picker
// for them would be building a worse keyboard.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    property string eventId
    /// The reaction the user settled on, read by whoever pushed this.
    property string key: ""

    readonly property var common: ["👍", "👎", "❤️", "😂", "🎉", "😮", "😢", "🙏"]

    canAccept: key.length > 0

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            DialogHeader {
                acceptText: qsTr("React")
            }

            Flow {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                spacing: Theme.paddingMedium

                Repeater {
                    model: dialog.common

                    BackgroundItem {
                        width: Theme.itemSizeSmall
                        height: Theme.itemSizeSmall
                        highlighted: down || dialog.key === modelData
                        onClicked: {
                            dialog.key = modelData
                            dialog.accept()
                        }

                        // The same rule as in the conversation: a file the
                        // user supplied, otherwise the character itself.
                        property string picture: settings.emojiImages
                                                 ? matrix.emojiSource(modelData)
                                                 : ""

                        Image {
                            anchors.centerIn: parent
                            visible: parent.picture.length > 0 && status !== Image.Error
                            source: parent.picture
                            sourceSize.width: Theme.iconSizeMedium
                            sourceSize.height: Theme.iconSizeMedium
                            width: Theme.iconSizeMedium
                            height: Theme.iconSizeMedium
                            asynchronous: true
                        }

                        Label {
                            anchors.centerIn: parent
                            visible: parent.picture.length === 0
                            font.pixelSize: Theme.fontSizeLarge
                            textFormat: Text.PlainText
                            text: modelData
                        }
                    }
                }
            }

            TextField {
                width: parent.width
                label: qsTr("Something else")
                placeholderText: qsTr("Something else")
                inputMethodHints: Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: {
                    if (text.trim().length > 0) {
                        dialog.key = text.trim()
                        dialog.accept()
                    }
                }
                onTextChanged: dialog.key = text.trim()
            }
        }
    }
}
