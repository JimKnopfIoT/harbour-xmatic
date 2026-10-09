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
    readonly property var gatewayModes: ["server", "public", "other"]
    // "Other" picked in the list before an address was entered.
    property bool choosingOther: false
    readonly property string gatewayMode: choosingOther ? "other" : settings.pushGatewayMode
    // true, false, or null while unknown: only a registered address can be asked.
    readonly property var serverGateway: pushStatus.serverGateway === undefined
                                         ? null : pushStatus.serverGateway

    function host(url) {
        return String(url).replace(/^https:\/\//i, "").split("/")[0]
    }

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

            ComboBox {
                id: gatewayBox

                width: parent.width
                label: qsTr("Gateway")
                currentIndex: page.gatewayModes.indexOf(page.gatewayMode)
                value: [qsTr("Push server's own"), qsTr("UnifiedPush public gateway"),
                        qsTr("Other")][currentIndex] || qsTr("Not chosen")
                description: {
                    switch (page.gatewayMode) {
                    case "server":
                        if (page.serverGateway === true) {
                            return qsTr("%1, the server that already holds this device's address.")
                                .arg(page.host(page.pushStatus.serverGatewayUrl))
                        }
                        if (page.serverGateway === false) {
                            return qsTr("Your push server has no Matrix gateway. Choose another one.")
                        }
                        return qsTr("Found once push is on. ntfy servers have one; the Mozilla service does not.")
                    case "public":
                        return qsTr("Run by the UnifiedPush project. It sees which room every notification is for.")
                    case "other":
                        return qsTr("It sees which room every notification is for.")
                    }
                    return qsTr("Your homeserver posts to a Matrix gateway, which forwards to this device. Choose one to turn push on.")
                }

                menu: ContextMenu {
                    MenuItem {
                        text: qsTr("Push server's own")
                        enabled: page.serverGateway !== false
                        onClicked: {
                            page.choosingOther = false
                            matrix.setPushGateway("server")
                        }
                    }
                    MenuItem {
                        text: qsTr("UnifiedPush public gateway")
                        onClicked: {
                            page.choosingOther = false
                            matrix.setPushGateway("public")
                        }
                    }
                    MenuItem {
                        text: qsTr("Other")
                        onClicked: {
                            page.choosingOther = settings.pushGatewayMode !== "other"
                            gatewayField.forceActiveFocus()
                        }
                    }
                }
            }

            TextField {
                id: gatewayField

                visible: page.gatewayMode === "other"
                width: parent.width
                text: settings.pushGateway
                label: qsTr("Gateway address")
                placeholderText: "https://example.org/_matrix/push/v1/notify"
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText | Qt.ImhUrlCharactersOnly
                EnterKey.enabled: /^https:\/\/./i.test(text.trim())
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: {
                    if (matrix.setPushGateway("other", text)) {
                        page.choosingOther = false
                        focus = false
                    }
                }
            }

            TextSwitch {
                text: qsTr("Receive push notifications")
                checked: page.pushOn
                automaticCheck: false
                busy: page.pushState === "registering"
                // Off must stay possible without a distributor or a gateway.
                enabled: page.pushOn || (page.distributors.length > 0
                                         && settings.pushGatewayMode.length > 0)
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
                visible: page.pushOn
                label: qsTr("Gateway")
                level: page.gateway.length > 0 ? SecurityStatus.GREEN : SecurityStatus.RED
                detail: page.gateway.length > 0
                        ? page.host(page.gateway)
                        : qsTr("None yet; until one is chosen your homeserver cannot reach this device.")
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
