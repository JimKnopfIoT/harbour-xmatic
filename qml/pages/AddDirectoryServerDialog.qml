import QtQuick 2.0
import Sailfish.Silica 1.0

// Asks for one directory server. What the user pastes is reduced to the bare
// name the directory API expects — a scheme or a trailing slash is dropped.
Dialog {
    id: dialog

    property string serverName: normalized(nameField.text)

    canAccept: serverName.indexOf(".") > 0

    function normalized(text) {
        var name = text.trim().toLowerCase()
        if (name.indexOf("https://") === 0) {
            name = name.substring(8)
        } else if (name.indexOf("http://") === 0) {
            name = name.substring(7)
        }
        while (name.length > 0 && name.charAt(name.length - 1) === "/") {
            name = name.substring(0, name.length - 1)
        }
        return name
    }

    Column {
        width: parent.width

        DialogHeader {
            acceptText: qsTr("Add")
        }

        TextField {
            id: nameField

            width: parent.width
            focus: true
            label: qsTr("Directory server")
            placeholderText: qsTr("For example matrix.org")
            inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                              | Qt.ImhUrlCharactersOnly
            EnterKey.enabled: dialog.canAccept
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: dialog.accept()
        }
    }
}
