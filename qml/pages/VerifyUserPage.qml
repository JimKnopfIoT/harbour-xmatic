import QtQuick 2.0
import Sailfish.Silica 1.0

// Verifying another person by their Matrix address.
//
// The way in for everyone this account has no two-party encrypted chat with —
// a group room, or somebody not written to yet. Where such a chat does exist,
// the room's own menu offers the same thing without asking for an address,
// because that is the case where the app already knows it.
//
// This used to be a field on the encryption page, directly under the button
// that verifies one's *own* second device. The two read as variants of each
// other there, and a new user took the wrong one; here the question is only
// ever about a person.
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

            // Two examples rather than one: the shape is "@name:server", and a
            // single line of it reads as the one address that works. The
            // domains are the ones reserved for documentation, so neither can
            // be somebody's real account.
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

            // The other kind, in the same place and in the same order it has
            // on the encryption page: a person first, then a device of one's
            // own. Both were once side by side under one heading, which is how
            // a new user came to press the device one meaning to verify
            // somebody else. Apart, each says what it is - and whoever comes
            // here for verification finds both rather than one and a hint
            // that the other lives somewhere else.
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
