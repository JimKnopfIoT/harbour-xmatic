import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// Push notifications through a UnifiedPush distributor.
//
// This app has no background service by decision, so nothing arrives while it
// is closed. UnifiedPush does not change that decision — the *distributor* is
// the daemon, one per device, shared by every app on it. This page is where
// that arrangement is turned on and where it says what it is doing.
//
// The switch lives here rather than in the account list on purpose. A switch
// in a list that flips to on and then does nothing, because no distributor is
// installed, is a state with no visible action — the shape that has cost this
// app a dead pull-down twice. Here the switch and its consequence are one
// screen: flip it, and the line underneath says in red what is missing.
Page {
    id: page

    allowedOrientations: Orientation.All

    readonly property var pushStatus: matrix.pushStatus
    readonly property string pushState: pushStatus.state || ""
    readonly property var distributors: pushStatus.distributors || []

    Component.onCompleted: matrix.refreshPushStatus()

    // On every visit, not once: a distributor can be installed or removed
    // while this app runs, and a page answering from a cache would be wrong
    // exactly then.
    onStatusChanged: {
        if (status === PageStatus.Active) {
            matrix.refreshPushStatus()
        }
    }

    function apply(on) {
        settings.pushEnabled = on
        if (on) {
            matrix.enablePush(settings.pushGateway)
        } else {
            matrix.disablePush()
        }
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
                title: qsTr("Push notifications")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("xmatic has no background service, so messages arrive only while it runs. A push distributor is a separate app that holds one connection for every app on the device and wakes them when something comes in.")
            }

            TextSwitch {
                text: qsTr("Receive push notifications")
                checked: settings.pushEnabled
                automaticCheck: false
                enabled: settings.pushGateway.trim().length > 0
                onClicked: page.apply(!settings.pushEnabled)
            }

            SecurityRow {
                label: qsTr("Distributor")
                level: page.distributors.length > 0 ? SecurityStatus.GREEN
                                                    : SecurityStatus.RED
                detail: page.distributors.length > 0
                        // The bus name's last segment: the whole name is
                        // `org.unifiedpush.Distributor.<something>` and only
                        // the tail names the app.
                        ? String(page.distributors[0]).split(".").pop()
                        : qsTr("No push distributor is installed. Without one there is nothing to hold the connection, and this stays off.")
            }

            SecurityRow {
                label: qsTr("Registration")
                level: matrix.pushEndpointReady
                       ? SecurityStatus.GREEN
                       : (page.pushState === "registering" ? SecurityStatus.ORANGE
                                                       : SecurityStatus.RED)
                detail: {
                    if (matrix.pushEndpointReady) {
                        return qsTr("This device has an address to be reached at.")
                    }
                    if (page.pushState === "registering") {
                        return qsTr("Waiting for the distributor.")
                    }
                    return qsTr("Not registered.")
                }
            }

            SectionHeader {
                text: qsTr("Gateway")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("A Matrix homeserver cannot talk to a push distributor directly, so it posts to a gateway that forwards. There is no default: it is the one thing nobody can guess for you.")
            }

            TextField {
                id: gatewayField

                width: parent.width
                text: settings.pushGateway
                label: qsTr("Push gateway")
                placeholderText: "https://example.org/_matrix/push/v1/notify"
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: {
                    settings.pushGateway = text
                    focus = false
                }
            }

            SectionHeader {
                text: qsTr("What leaves this device")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Your homeserver is told an address at the push service, and posts a room and message identifier to the gateway for every notification. No message text: the push carries identifiers only and this device fetches and decrypts the message itself. That address is a secret — whoever holds it can send this phone a notification.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                visible: (page.pushStatus.error || "").length > 0
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                text: page.pushStatus.error || ""
            }
        }
    }
}
