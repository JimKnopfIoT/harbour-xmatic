import QtQuick 2.0
import Sailfish.Silica 1.0

// Why the secrets service hands out no key. The advice follows the measurement.
Page {
    id: page

    allowedOrientations: Orientation.All

    // Asked once on open. Never interactive, so it cannot hang on a dialog.
    property var diagnosis: ({})

    // Corrupted before locked: damaged data reports itself locked too, and no
    // code unlocks that.
    readonly property string reason: {
        if (!matrix.secretsDaemonPresent) {
            return "noDaemon"
        }
        if (diagnosis.masterlockHealth === 2 || diagnosis.saltDataHealth === 2) {
            return "corrupted"
        }
        if (diagnosis.lockStatus === 3) {
            return "daemonLocked"
        }
        if (diagnosis.collection === 1) {
            return "collectionLocked"
        }
        return "unknown"
    }

    // Empty where there is nothing to type.
    readonly property string command:
        reason === "noDaemon"
        ? "devel-su pkcon install sailfishsecretsdaemon sailfishsecretsdaemon-secretsplugin-common"
        : reason === "collectionLocked"
          ? "devel-su pkcon install sailfish-components-secrets-ui"
          : ""

    Component.onCompleted: page.diagnosis = matrix.secretsDiagnosis()

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingLarge

            PageHeader {
                title: qsTr("Getting in again")
            }

            Row {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                spacing: Theme.paddingMedium

                SecurityLamp {
                    level: page.reason === "collectionLocked" ? "orange" : "red"
                    anchors.verticalCenter: parent.verticalCenter
                }

                Label {
                    width: parent.width - Theme.iconSizeExtraSmall / 2 - Theme.paddingMedium
                    wrapMode: Text.Wrap
                    color: Theme.highlightColor
                    text: {
                        switch (page.reason) {
                        case "noDaemon":
                            return qsTr("This system is missing the service that keeps encryption keys.")
                        case "corrupted":
                            return qsTr("The device's secrets service reports damaged data. Nothing can be unlocked in this state.")
                        case "daemonLocked":
                            return qsTr("The device's secrets service itself is locked. Every app on this phone that stores secrets is affected, not only xmatic.")
                        case "collectionLocked":
                            return qsTr("xmatic's own key store is locked and waiting for your confirmation.")
                        default:
                            return qsTr("The secrets service did not hand out a key, and it did not say why.")
                        }
                    }
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: {
                    switch (page.reason) {
                    case "noDaemon":
                        return qsTr("This is a property of the operating system, not a fault in xmatic. The package exists and can be installed; some Sailfish images simply do not ship it.")
                    case "corrupted":
                        return qsTr("This is beyond what xmatic can reach. Repairing it means resetting the service's own data, which affects every app on the device that stores secrets — ask in the Sailfish support channels before doing that.")
                    case "daemonLocked":
                        return qsTr("The service is unlocked at start-up from your device lock code. Where that hand-off has stopped working, restarting lands in the same state every time — setting the code afresh is what starts it again.")
                    case "collectionLocked":
                        return qsTr("The approval lasts until the device is restarted. If no request appears at all, the system components that draw it may be missing.")
                    default:
                        return qsTr("Setting the device lock code afresh and restarting is worth trying: it is what unlocks the service at start-up.")
                    }
                }
            }

            SectionHeader {
                text: qsTr("What to do")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.primaryColor
                text: {
                    switch (page.reason) {
                    case "noDaemon":
                        return qsTr("1. Switch on Developer mode in the system settings, under Settings › Developer tools.")
                               + "\n\n"
                               + qsTr("2. Open the Terminal app and run the line below — tap it to copy. Then restart the device and start xmatic again.")
                    case "corrupted":
                        return qsTr("Note down what this page reports below and take it to the Sailfish support channels. Signing out of xmatic does not repair it.")
                    case "collectionLocked":
                        return qsTr("Go back and tap “Try again”, then confirm the system's request. If nothing appears, install the missing components (tap the line to copy) and restart the device.")
                    default:
                        return qsTr("1. Open Settings › Device lock and set a new security code.")
                               + "\n\n"
                               + qsTr("2. Restart the device.")
                               + "\n\n"
                               + qsTr("3. Start xmatic again.")
                    }
                }
            }

            BackgroundItem {
                width: parent.width
                height: commandLine.height + 2 * Theme.paddingMedium
                visible: page.command.length > 0
                onClicked: Clipboard.text = page.command

                Label {
                    id: commandLine

                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    // At the spaces: a command broken mid-word gets typed wrongly.
                    wrapMode: Text.Wrap
                    font.family: "monospace"
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.highlightColor
                    text: page.command
                }
            }

            // Nothing is on disk on the blocked page; the warning would invent
            // a risk there.
            SectionHeader {
                visible: matrix.sessionState !== "none"
                text: qsTr("Signing out does not help")
            }

            Label {
                visible: matrix.sessionState !== "none"
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.primaryColor
                text: qsTr("Signing out deletes this device's encryption keys along with the local data. You would then have to verify this device again from another one, and older messages would need your recovery key. It does not repair the secrets service.")
            }

            SectionHeader {
                text: qsTr("What this page measured")
            }

            // Verbatim, for a report: the words above only interpret them.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.WrapAnywhere
                font.family: "monospace"
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: "service: " + (matrix.secretsDaemonPresent ? "yes" : "no")
                      + "\nmetadata lock: " + page.diagnosis.lockStatus
                      + "\nmaster lock health: " + page.diagnosis.masterlockHealth
                      + "\nsalt health: " + page.diagnosis.saltDataHealth
                      + "\nown collection: " + page.diagnosis.collection
                      + "\nerror: " + page.diagnosis.errorCode
                      + (page.diagnosis.errorMessage
                         ? "\n" + page.diagnosis.errorMessage : "")
            }
        }

        VerticalScrollDecorator { }
    }
}
