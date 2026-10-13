import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus
import "Strings.js" as Strings

// Push through a UnifiedPush distributor - the daemon is the distributor, this
// is where the arrangement is turned on and says what it is doing.
Page {
    id: page

    allowedOrientations: Orientation.All

    readonly property var pushStatus: matrix.push.status
    readonly property string pushState: pushStatus.state || ""
    readonly property var distributors: pushStatus.distributors || []
    readonly property bool pushOn: matrix.push.endpointReady
    // The bus name, while the connector runs; "" before a pick.
    readonly property string chosenDistributor: pushStatus.distributor || ""
    readonly property bool choosingDistributor: pushState === "choose-distributor"
    readonly property string gateway: pushStatus.gateway || ""
    readonly property var gatewayModes: ["server", "public", "other"]
    // "Other" picked in the list before an address was entered.
    property bool choosingOther: false
    // Switched on but no gateway picked yet.
    property bool wanted: false
    readonly property bool switchedOn: pushOn || wanted || pushState === "registering"
    readonly property string gatewayMode: choosingOther ? "other" : settings.pushGatewayMode
    // "yes", "no", "unknown", or "" before registering.
    readonly property string serverGateway: pushStatus.serverGateway || ""
    readonly property string distributorName: chosenDistributor.length > 0
                                              ? tail(chosenDistributor)
                                              : (distributors.length === 1 ? tail(distributors[0])
                                                                           : qsTr("the distributor"))
    // Set whenever the gateway answer is: both come with the push address.
    readonly property string pushService: pushStatus.pushService || ""
    readonly property bool serviceLacksGateway: pushOn && gatewayMode === "server"
                                                && serverGateway === "no"

    onGatewayModeChanged: gatewayBox.currentIndex = gatewayModes.indexOf(gatewayMode)

    // A bus name's last segment: `org.unifiedpush.Distributor.<app>` names the app last.
    function tail(name) {
        return String(name).split(".").pop()
    }

    function host(url) {
        return String(url).replace(/^https:\/\//i, "").split("/")[0]
    }

    Component.onCompleted: {
        gatewayBox.currentIndex = gatewayModes.indexOf(gatewayMode)
        matrix.push.refresh()
    }

    // On every visit, not once: a distributor can be installed or removed while
    // this app runs, and a cached answer would be wrong exactly then.
    onStatusChanged: {
        if (status === PageStatus.Active) {
            matrix.push.refresh()
        }
    }

    function apply(on) {
        if (!on) {
            wanted = false
            choosingOther = false
            // Clearing the pick drops the registration; without one, disable does.
            if (settings.pushGatewayMode.length > 0) {
                matrix.push.setGateway("")
            } else if (pushOn || pushState === "registering") {
                matrix.push.disable()
            }
        } else if (settings.pushGatewayMode.length > 0) {
            // Several distributors and none picked: the pick finishes it.
            wanted = distributors.length > 1 && chosenDistributor.length === 0
            matrix.push.enable()
        } else {
            wanted = true
        }
    }

    // A new pick drops the registration and, if complete, registers again.
    function pick(mode, address) {
        if (!matrix.push.setGateway(mode, address, wanted || pushOn)) {
            return false
        }
        choosingOther = false
        wanted = wanted || pushOn
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
                id: distributorBox

                visible: page.distributors.length > 1
                width: parent.width
                label: qsTr("Distributor")
                automaticSelection: false
                currentIndex: -1
                value: page.chosenDistributor.length > 0 ? page.tail(page.chosenDistributor)
                                                         : qsTr("None")
                description: page.choosingDistributor
                             ? qsTr("Several distributors are installed. Pick the one xmatic uses.")
                             : ""

                menu: ContextMenu {
                    Repeater {
                        model: page.distributors

                        MenuItem {
                            text: page.tail(modelData)
                            onClicked: matrix.push.selectDistributor(modelData, page.switchedOn)
                        }
                    }
                }
            }

            ComboBox {
                id: gatewayBox

                visible: page.switchedOn
                width: parent.width
                label: qsTr("Gateway")
                // Set from the saved pick only; Silica's own selection breaks a binding.
                automaticSelection: false
                currentIndex: -1
                value: [qsTr("Provided"), qsTr("UnifiedPush (public)"),
                        qsTr("Custom")][page.gatewayModes.indexOf(page.gatewayMode)] || qsTr("None")
                description: {
                    switch (page.gatewayMode) {
                    case "server":
                        if (page.serverGateway === "yes") {
                            // The distributor names itself: one pass, so its name stays a name.
                            return Strings.fill(qsTr("Provided via %1: %2"),
                                                [page.distributorName,
                                                 page.host(page.pushStatus.serverGatewayUrl)])
                        }
                        if (page.serverGateway === "no") {
                            return qsTr("%1 has no Matrix gateway.").arg(page.pushService)
                        }
                        if (page.serverGateway === "unknown") {
                            return qsTr("No answer from %1 yet. Asking again.").arg(page.pushService)
                        }
                        return qsTr("Found via %1 after registering.").arg(page.distributorName)
                    case "public":
                        return qsTr("Run by the UnifiedPush project: %1").arg("matrix.gateway.unifiedpush.org")
                    case "other":
                        return ""
                    }
                    return qsTr("Pick a gateway to finish turning push on.")
                }

                menu: ContextMenu {
                    MenuItem {
                        text: qsTr("Provided")
                        onClicked: page.pick("server", "")
                    }
                    MenuItem {
                        text: qsTr("UnifiedPush (public)")
                        onClicked: page.pick("public", "")
                    }
                    MenuItem {
                        text: qsTr("Custom")
                        onClicked: {
                            if (settings.pushGatewayMode !== "other") {
                                page.pick("", "")
                                page.choosingOther = true
                            }
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

            Column {
                visible: page.serviceLacksGateway
                width: parent.width
                spacing: Theme.paddingMedium

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    color: Theme.errorColor
                    text: qsTr("%1 can't deliver Matrix notifications.").arg(page.pushService)
                }

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.secondaryColor
                    text: qsTr("Choose a push service with a Matrix gateway in %1, or use the public gateway run by the UnifiedPush project.").arg(page.distributorName)
                }

                Button {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: qsTr("Use UnifiedPush public gateway")
                    onClicked: page.pick("public", "")
                }
            }

            SecurityRow {
                label: qsTr("Distributor")
                level: page.distributors.length === 0
                       ? SecurityStatus.RED
                       : (page.choosingDistributor ? SecurityStatus.ORANGE : SecurityStatus.GREEN)
                detail: {
                    if (page.distributors.length === 0) {
                        return page.pushOn
                                ? qsTr("The push distributor is no longer installed. Nothing arrives until it is back. Install it again or switch push off.")
                                : qsTr("No push distributor is installed. Without one there is nothing to hold the connection, and this stays off.")
                    }
                    if (page.chosenDistributor.length > 0 || page.distributors.length === 1) {
                        return page.distributorName
                    }
                    return qsTr("Several distributors are installed. Pick the one xmatic uses.")
                }
            }

            SecurityRow {
                label: qsTr("Registration")
                // Registered with nothing left to deliver is not green.
                level: page.pushStatus.registered && page.pushState !== "no-distributor"
                       ? SecurityStatus.GREEN
                       : (page.pushOn || page.pushState === "registering"
                          ? SecurityStatus.ORANGE : SecurityStatus.RED)
                detail: {
                    if (page.pushStatus.registered) {
                        return qsTr("This device has an address to be reached at.")
                    }
                    if (page.serviceLacksGateway) {
                        return qsTr("Registered with %1. The homeserver can't reach it.").arg(page.distributorName)
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
                text: qsTr("Your homeserver is told an address at the push service, and posts a room and message identifier and your unread count to the gateway for every notification. No message text: this device fetches and decrypts the message itself. That address is a secret — whoever holds it can send this phone a notification.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Before you uninstall xmatic, switch push off here. Otherwise the homeserver keeps the pusher and goes on posting to the gateway.")
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
