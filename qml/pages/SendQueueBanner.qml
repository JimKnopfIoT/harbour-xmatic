import QtQuick 2.0
import Sailfish.Silica 1.0

// The room's parked send request: it holds back everything after it.
Column {
    id: banner

    // The one that blocks the rest; the others wait behind it.
    readonly property var head: matrix.sendQueue.stuck.length > 0
                                ? matrix.sendQueue.stuck[0] : null
    property bool acting: false

    visible: head !== null
    height: visible ? implicitHeight : 0
    spacing: Theme.paddingSmall

    function what(kind) {
        switch (kind) {
        case "message": return qsTr("A message could not be sent.")
        case "edit": return qsTr("An edit could not be sent.")
        case "deletion": return qsTr("A deletion could not be sent. The message is still there for everyone else.")
        case "reaction": return qsTr("A reaction could not be sent.")
        case "vote": return qsTr("A poll vote could not be sent.")
        case "poll": return qsTr("A poll could not be sent.")
        default: return qsTr("A change could not be sent.")
        }
    }

    function why(row) {
        switch (row.reason) {
        case "identityViolation":
            return qsTr("Someone you verified has a new identity. Withdraw the verification on their profile, then send again.")
        case "ownVerification":
            return qsTr("This device has to be verified first.")
        case "insecureDevices":
            return qsTr("Some devices in this room are not verified.")
        default:
            return row.detail || ""
        }
    }

    // Every answer lifts the buttons.
    Connections {
        target: matrix.sendQueue
        onStuckChanged: banner.acting = false
        onFailed: banner.acting = false
    }

    Item {
        width: parent.width
        height: Theme.paddingSmall
    }

    Label {
        x: Theme.horizontalPageMargin
        width: parent.width - 2 * Theme.horizontalPageMargin
        wrapMode: Text.Wrap
        font.pixelSize: Theme.fontSizeSmall
        color: Theme.highlightColor
        text: banner.head
              ? banner.what(banner.head.kind) + " "
                + qsTr("Everything sent after it in this room waits behind it.")
              : ""
    }

    Label {
        x: Theme.horizontalPageMargin
        width: parent.width - 2 * Theme.horizontalPageMargin
        wrapMode: Text.Wrap
        font.pixelSize: Theme.fontSizeExtraSmall
        color: Theme.secondaryHighlightColor
        visible: text.length > 0
        text: banner.head ? banner.why(banner.head) : ""
    }

    // Widths from the banner, no Row: width rule.
    Item {
        width: parent.width
        height: sendAgain.height

        Button {
            id: sendAgain

            anchors {
                left: parent.left
                leftMargin: Theme.horizontalPageMargin
            }
            width: (parent.width - 2 * Theme.horizontalPageMargin - Theme.paddingMedium) / 2
            // A reaction is withdrawn by its chip, not resent.
            enabled: !banner.acting && banner.head !== null && banner.head.kind !== "reaction"
            text: qsTr("Send again")
            onClicked: {
                banner.acting = true
                matrix.sendQueue.retry(banner.head.txnId)
            }
        }

        Button {
            anchors {
                right: parent.right
                rightMargin: Theme.horizontalPageMargin
            }
            width: sendAgain.width
            enabled: !banner.acting && banner.head !== null
            text: qsTr("Discard")
            onClicked: {
                banner.acting = true
                matrix.sendQueue.discard(banner.head.txnId)
            }
        }
    }

    Item {
        width: parent.width
        height: Theme.paddingSmall
    }
}
