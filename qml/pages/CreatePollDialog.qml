import QtQuick 2.0
import Sailfish.Silica 1.0

// A poll is written once and cannot be changed afterwards: kind and the number
// of answers travel with the start event.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    /// Twenty is the spec's ceiling; two is what makes a poll a poll.
    readonly property int maxAnswers: 20

    property int answerCount: 3
    /// Recounted whenever a field changes - `canAccept` needs it live.
    property int filledAnswers: 0

    // The word being typed sits in the input method's preedit rather than in
    // `text`, so a question plainly on screen would leave the button grey.
    canAccept: (questionField.text.trim().length > 0 || questionField.inputMethodComposing)
               && filledAnswers >= 2

    function recount() {
        var filled = 0
        for (var i = 0; i < dialog.answerCount; i++) {
            var field = answerRepeater.itemAt(i)
            // The word being typed is in the preedit, not in `text` - the
            // same rule the question follows.
            if (field && (field.text.trim().length > 0 || field.inputMethodComposing)) {
                filled++
            }
        }
        dialog.filledAnswers = filled
    }

    onAccepted: {
        Qt.inputMethod.commit()
        var answers = []
        for (var i = 0; i < dialog.answerCount; i++) {
            var field = answerRepeater.itemAt(i)
            var text = field ? field.text.trim() : ""
            if (text.length > 0) {
                answers.push(text)
            }
        }
        matrix.polls.create(questionField.text.trim(), answers,
                          hideSwitch.checked,
                          multiSwitch.checked ? answers.length : 1)
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            DialogHeader {
                acceptText: qsTr("Create poll")
            }

            TextField {
                id: questionField

                width: parent.width
                label: qsTr("Question")
                placeholderText: qsTr("What are you asking?")
                focus: true
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: focus = false
            }

            Repeater {
                id: answerRepeater

                model: dialog.answerCount

                TextField {
                    width: content.width
                    label: qsTr("Answer %1").arg(index + 1)
                    placeholderText: qsTr("Answer %1").arg(index + 1)
                    EnterKey.iconSource: "image://theme/icon-m-enter-next"
                    EnterKey.onClicked: focus = false
                    onTextChanged: dialog.recount()
                    onInputMethodComposingChanged: dialog.recount()
                }
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Add answer")
                enabled: dialog.answerCount < dialog.maxAnswers
                onClicked: dialog.answerCount++
            }

            TextSwitch {
                id: multiSwitch

                text: qsTr("Allow several answers")
                description: qsTr("Everyone can then pick more than one answer.")
            }

            TextSwitch {
                id: hideSwitch

                text: qsTr("Hide results until the poll ends")
                description: qsTr("Clients keep the counts hidden until the poll ends, yours included. A convention every common client follows, not a lock.")
            }

            // Said where the poll is made, not in a document nobody opens: the
            // room keeps every vote as an event, and no client can change that.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("A poll is not a secret ballot: who voted for what stays readable in the room.")
            }
        }
    }
}
