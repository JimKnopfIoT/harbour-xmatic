import QtQuick 2.0
import Sailfish.Silica 1.0

// Ban by address, member or not: a ban also keeps out someone who is not in
// the room yet. The core checks the address; this only gates the obvious.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    property string roomId
    property string roomName

    readonly property string address: userField.text.trim()
    readonly property bool plausible: /^@[^\s:]+:[^\s]+$/.test(address)

    canAccept: plausible

    onAccepted: {
        Qt.inputMethod.commit()
        matrix.bans.ban(roomId, userField.text.trim(), reasonField.text.trim())
    }

    // Landscape leaves little more than the header; the fields must scroll.
    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height + Theme.paddingLarge

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            DialogHeader {
                acceptText: qsTranslate("MemberProfilePage", "Ban")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                color: Theme.secondaryColor
                textFormat: Text.PlainText
                visible: dialog.roomName.length > 0
                text: dialog.roomName
            }

            TextField {
                id: userField

                width: parent.width
                label: qsTranslate("InviteToRoomDialog", "User ID")
                placeholderText: "@name:server"
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                focus: true
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: reasonField.forceActiveFocus()
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.errorColor
                visible: dialog.address.length > 0 && !dialog.plausible
                text: qsTr("A Matrix address looks like @name:server.")
            }

            TextField {
                id: reasonField

                width: parent.width
                label: qsTr("Reason (optional)")
                placeholderText: qsTr("Reason (optional)")
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: {
                    if (dialog.canAccept) {
                        dialog.accept()
                    }
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("The person can no longer enter this room. If they are a member now, they are removed. Everyone in the room sees the ban and its reason.")
            }
        }
    }
}
