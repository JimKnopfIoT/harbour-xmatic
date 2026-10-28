import QtQuick 2.0
import Sailfish.Silica 1.0

// Any power level up to one's own; negative ones take away what the room
// grants everyone. The server checks again.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    property string roomId
    property string userId
    property string subject
    property int current: 0
    property int ownPower: 0

    readonly property int level: parseInt(levelField.text, 10)
    readonly property bool valid: levelField.acceptableInput && !isNaN(level)

    canAccept: valid && level !== current

    onAccepted: matrix.setMemberPower(roomId, userId, level)

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            DialogHeader {
                acceptText: qsTr("Set")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.secondaryColor
                textFormat: Text.PlainText
                visible: dialog.subject.length > 0
                text: dialog.subject
            }

            TextField {
                id: levelField

                width: parent.width
                label: qsTr("Power level")
                text: String(dialog.current)
                inputMethodHints: Qt.ImhFormattedNumbersOnly | Qt.ImhNoPredictiveText
                validator: IntValidator {
                    bottom: -2147483647
                    top: dialog.ownPower
                }
                focus: true
                EnterKey.enabled: dialog.canAccept
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: dialog.accept()
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("0 is a member, 50 a moderator, 100 an admin. Below 0 they lose what the room allows everyone. At most your own level, %1.").arg(dialog.ownPower)
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.errorColor
                visible: dialog.valid && dialog.level >= dialog.ownPower
                text: qsTr("At your own level this cannot be taken back: only they themselves can lower it afterwards.")
            }
        }
    }
}
