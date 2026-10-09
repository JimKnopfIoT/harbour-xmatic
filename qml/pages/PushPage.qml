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
    // Switched on but no gateway picked yet.
    property bool wanted: false
    readonly property bool switchedOn: pushOn || wanted || pushState === "registering"
    readonly property string gatewayMode: choosingOther ? "other" : settings.pushGatewayMode
    // true, false, or null if not checked yet.
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
        if (!on) {
            wanted = false
            choosingOther = false
            if (pushOn || pushState === "registering") {
                matrix.disablePush()
            }
        } else if (settings.pushGatewayMode.length > 0) {
            matrix.enablePush()
        } else {
            wanted = true
        }
    }

    // The registration starts once a gateway is picked.
    function pick(mode, address) {
        if (!matrix.setPushGateway(mode, address)) {
            return false
        }
        choosingOther = false
        if (wanted && !pushOn) {
            wanted = false
            matrix.enablePush()
        }
        return true
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
                checked: page.switchedOn
                automaticCheck: false
                busy: page.pushState === "registering"
                // Off must stay possible without a distributor.
                enabled: page.switchedOn || page.distributors.length > 0
                onClicked: page.apply(!page.switchedOn)
            }

            ComboBox {
                id: gatewayBox

                visible: page.switchedOn
                width: parent.width
                label: qsTr("Gateway")
                currentIndex: page.gatewayModes.indexOf(page.gatewayMode)
                value: [qsTr("Push server"), qsTr("UnifiedPush (public)"),
                        qsTr("Custom")][currentIndex] || qsTr("None")
                description: {
                    switch (page.gatewayMode) {
                    case "server":
                        if (page.serverGateway === true) {
                            return qsTr("Uses %1.").arg(page.host(page.pushStatus.serverGatewayUrl))
                        }
                        if (page.serverGateway === false) {
                            return qsTr("Your push server has no gateway. Pick another one.")
                        }
                        return qsTr("Checked after registering. ntfy has one, Mozilla doesn't.")
                    case "public":
                        return "matrix.gateway.unifiedpush.org"
                    case "other":
                        return ""
                    }
                    return qsTr("Pick a gateway to finish turning push on.")
                }

                menu: ContextMenu {
                    MenuItem {
                        text: qsTr("Push server")
                        enabled: page.serverGateway !== false
                        onClicked: page.pick("server", "")
                    }
                    MenuItem {
                        text: qsTr("UnifiedPush (public)")
                        onClicked: page.pick("public", "")
                    }
                    MenuItem {
                        text: qsTr("Custom")
                        onClicked: {
                            page.choosingOther = settings.pushGatewayMode !== "other"
                            gatewayField.forceActiveFocus()
                        }
                    }
                }
            }

            TextField {
                id: gatewayField

                visible: page.switchedOn && page.gatewayMode === "other"
                width: parent.width
                text: settings.pushGateway
                label: qsTr("Gateway URL")
                placeholderText: "https://example.org/_matrix/push/v1/notify"
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText | Qt.ImhUrlCharactersOnly
                EnterKey.enabled: /^https:\/\/./i.test(text.trim())
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: {
                    if (page.pick("other", text)) {
                        focus = false
                    }
                }
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
                        return qsTr("Registered. Waiting for a gateway.")
                    }
                    if (page.pushOn) {
                        return qsTr("Registered. Telling the homeserver.")
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
                        : qsTr("None. The homeserver can't reach this device yet.")
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
