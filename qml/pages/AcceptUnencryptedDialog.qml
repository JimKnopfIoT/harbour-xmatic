import QtQuick 2.0
import Sailfish.Silica 1.0

// The one way past the gate, and it has to be chosen rather than stumbled
// into. Accepting is remembered, so the question is asked once and not at
// every start; the state stays visible on the encryption page and in the lamp
// for as long as it lasts.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    onAccepted: {
        settings.acceptUnencryptedStorage()
        // The core was started without a key and there is nothing on disk, so
        // this only asks it for the session it will not find — which lands the
        // app on the login page, past the gate.
        matrix.restoreSession()
    }

    Column {
        width: parent.width
        spacing: Theme.paddingLarge

        DialogHeader {
            acceptText: qsTr("Continue without encryption")
            cancelText: qsTr("Back")
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            color: Theme.errorColor
            text: qsTr("Everything xmatic stores on this device stays readable: your session, your messages and your room keys.")
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.primaryColor
            text: qsTr("Choose this only if the service cannot be installed on this device. If you install it later, you can switch over on the encryption page — it costs one sign-out and your recovery key.")
        }

        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryColor
            text: qsTr("xmatic will keep showing this state under Encryption.")
        }
    }
}
