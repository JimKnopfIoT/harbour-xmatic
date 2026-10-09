import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// Push through a UnifiedPush distributor - the daemon is the distributor, this
// is where the arrangement is turned on and says what it is doing.
Page {
    id: page

    allowedOrientations: Orientation.All

    readonly property var pushStatus: matrix.pushStatus
    readonly property string pushState: pushStatus.state || ""
    readonly property var distributors: pushStatus.distributors || []
    readonly property bool pushOn: matrix.pushEndpointReady
    readonly property string gateway: pushStatus.gateway || ""
    readonly property string publicGateway: "https://matrix.gateway.unifiedpush.org/_matrix/push/v1/notify"

    Component.onCompleted: matrix.refreshPushStatus()

    // On every visit, not once: a distributor can be installed or removed while
    // this app runs, and a cached answer would be wrong exactly then.
    onStatusChanged: {
        if (status === PageStatus.Active) {
            matrix.refreshPushStatus()
        }
    }

    function apply(on) {
        if (on) {
            matrix.enablePush()
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
                checked: page.pushOn
                automaticCheck: false
                busy: page.pushState === "registering"
                // Off must stay possible without a distributor.
                enabled: page.pushOn || page.distributors.length > 0
                onClicked: page.apply(!page.pushOn)
            }

            SecurityRow {
                label: qsTr("Distributor")
                level: page.distributors.length > 0 ? SecurityStatus.GREEN
                                                    : SecurityStatus.RED
                detail: page.distributors.length > 0
                        // The bus name's last segment: the whole name is
                        // `org.unifiedpush.Distributor.<something>` and only the tail names the app.
                        ? String(page.distributors[0]).split(".").pop()
                        : qsTr("No push distributor is installed. Without one there is nothing to hold the connection, and this stays off.")
            }

            SecurityRow {
                label: qsTr("Registration")
                level: page.pushStatus.registered
                       ? SecurityStatus.GREEN
                       : (page.pushOn || page.pushState === "registering"
                          ? SecurityStatus.ORANGE : SecurityStatus.RED)
                detail: {
                    if (page.pushStatus.registered) {
                        return qsTr("This device has an address to be reached at.")
                    }
                    if (page.pushState === "needs-gateway") {
                        return qsTr("This device has an address; your homeserver needs a gateway to reach it.")
                    }
                    if (page.pushOn) {
                        return qsTr("This device has an address; waiting to tell your homeserver.")
                    }
                    if (page.pushState === "registering") {
                        return qsTr("Waiting for the distributor.")
                    }
                    return qsTr("Not registered.")
                }
            }

            SecurityRow {
                visible: page.pushOn || page.gateway.length > 0
                label: qsTr("Gateway")
                level: page.gateway.length > 0 ? SecurityStatus.GREEN : SecurityStatus.RED
                detail: page.gateway.length > 0
                        ? page.gateway.replace(/^https:\/\//i, "").split("/")[0]
                        : qsTr("Your push server has no Matrix gateway. Enter one below; until then your homeserver cannot reach this device.")
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
                text: qsTr("A Matrix homeserver cannot talk to a push distributor directly, so it posts to a gateway that forwards. Leave this empty to use your push server's own gateway, if it has one. The gateway sees which room every notification is for.")
            }

            TextField {
                id: gatewayField

                width: parent.width
                text: settings.pushGateway
                label: qsTr("Push gateway")
                placeholderText: "https://example.org/_matrix/push/v1/notify"
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText | Qt.ImhUrlCharactersOnly
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: {
                    if (matrix.setPushGateway(text)) {
                        focus = false
                    }
                }
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.pushOn && settings.pushGateway.length === 0
                         && !page.pushStatus.serverGateway
                text: qsTr("Use matrix.gateway.unifiedpush.org")
                onClicked: {
                    if (matrix.setPushGateway(page.publicGateway)) {
                        gatewayField.text = page.publicGateway
                    }
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
