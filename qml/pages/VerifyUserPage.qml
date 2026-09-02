import QtQuick 2.0
import Sailfish.Silica 1.0

// Verifying a person by address - the way in where no two-party encrypted chat
// exists. Where one does, the room's own menu asks for no address.
Page {
    id: page

    allowedOrientations: Orientation.All

    function verify() {
        var id = userField.text.trim()
        if (id.length === 0) {
            return
        }
        matrix.requestVerification(id)
        // Replaced, not stacked: this page was the way to the comparison and
        // has nothing to say afterwards.
        pageStack.replace(Qt.resolvedUrl("VerificationPage.qml"))
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        VerticalScrollDecorator {}

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: qsTr("Verify user")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Both sides then compare seven emoji. That is what says nobody is sitting in between — and it holds for every room you share with this person.")
            }

            TextField {
                id: userField

                width: parent.width
                label: qsTr("User ID")
                placeholderText: "@name:server"
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                focus: true
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.verify()
            }

            // Two examples rather than one: a single line reads as the one address that
            // works. The domains are the ones reserved for documentation.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("For example @anna:example.org or @tom:example.net")
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Verify")
                enabled: !matrix.encryptionBusy && userField.text.trim().length > 0
                onClicked: page.verify()
            }

            // A person first, then a device of one's own, in the same order as the
            // encryption page: side by side under one heading, a new user took the wrong one.
            SectionHeader {
                text: qsTr("Your own device")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Compares seven emoji with your other device. It needs that other device in front of you, and once both have confirmed, this one can read the shared room keys.")
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Verify this device")
                enabled: !matrix.encryptionBusy
                onClicked: {
                    matrix.requestVerification("")
                    pageStack.replace(Qt.resolvedUrl("VerificationPage.qml"))
                }
            }
        }
    }
}
