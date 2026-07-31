import QtQuick 2.0
import Sailfish.Silica 1.0

// Interactive verification: agree to the request, then compare seven emoji
// with the other screen. Confirming is what tells the crypto layer that no one
// sits in the middle — and, between one's own devices, what unlocks shared
// room keys.
Page {
    id: page

    objectName: "verificationPage"

    allowedOrientations: Orientation.All

    // Leaving the page mid-flow would strand the other side waiting.
    onStatusChanged: {
        if (status === PageStatus.Inactive
                && (matrix.verificationState === "done"
                    || matrix.verificationState === "cancelled")) {
            matrix.clearVerification()
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingLarge

            // No textFormat here: PageHeader has no such property, and
            // assigning it kills the whole page at load time ("Seite konnte
            // nicht geladen werden"). The description is a Matrix user id,
            // whose charset cannot carry markup anyway.
            PageHeader {
                title: qsTr("Verification")
                description: matrix.verificationIsSelf
                             ? qsTr("Another one of your devices")
                             : matrix.verificationUser
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: {
                    switch (matrix.verificationState) {
                    case "waiting":
                        return qsTr("Waiting for the other device to accept.")
                    case "requested":
                        // The side that asked cannot accept its own request;
                        // it waits for the other device to do so.
                        if (matrix.verificationWeStarted) {
                            return qsTr("Waiting for the other side to accept the request on their device.")
                        }
                        return matrix.verificationIsSelf
                                ? qsTr("Confirm that this is really your other device. Once verified, both can share room keys and older messages become readable.")
                                : qsTr("Confirm that you are really talking to this person and not to someone in between.")
                    case "comparing":
                        return qsTr("Both devices must show the same emoji, in the same order.")
                    case "done":
                        return qsTr("Verified.")
                    case "cancelled":
                        return qsTr("The verification was cancelled.")
                    default:
                        return ""
                    }
                }
            }

            // The short authentication string. Seven items, laid out in a grid
            // so they stay readable in both orientations.
            Grid {
                visible: matrix.verificationState === "comparing"
                anchors.horizontalCenter: parent.horizontalCenter
                columns: page.isPortrait ? 4 : 7
                spacing: Theme.paddingLarge

                Repeater {
                    model: matrix.verificationEmoji

                    Column {
                        width: Theme.itemSizeMedium
                        spacing: Theme.paddingSmall

                        Label {
                            anchors.horizontalCenter: parent.horizontalCenter
                            font.pixelSize: Theme.fontSizeExtraLarge
                            text: modelData.symbol
                        }

                        Label {
                            width: parent.width
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.Wrap
                            font.pixelSize: Theme.fontSizeTiny
                            color: Theme.secondaryColor
                            text: modelData.description
                        }
                    }
                }
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: matrix.verificationState === "requested"
                         && (matrix.verificationWeStarted || matrix.busy)
            }

            // Accept/decline belongs to the receiving side; the requester
            // only waits and gets its own cancel button below.
            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: Theme.paddingLarge
                visible: matrix.verificationState === "comparing"
                         || (matrix.verificationState === "requested"
                             && !matrix.verificationWeStarted)

                Button {
                    text: matrix.verificationState === "comparing"
                          ? qsTr("They do not match")
                          : qsTr("Decline")
                    onClicked: {
                        matrix.cancelVerification()
                        pageStack.pop()
                    }
                }

                Button {
                    text: matrix.verificationState === "comparing"
                          ? qsTr("They match")
                          : qsTr("Accept")
                    onClicked: {
                        if (matrix.verificationState === "comparing") {
                            matrix.confirmVerification()
                        } else {
                            matrix.acceptVerification()
                        }
                    }
                }
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: matrix.verificationState === "requested"
                         && matrix.verificationWeStarted
                text: qsTr("Cancel")
                onClicked: {
                    matrix.cancelVerification()
                    pageStack.pop()
                }
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: matrix.verificationState === "done"
                         || matrix.verificationState === "cancelled"
                text: qsTr("Close")
                onClicked: {
                    matrix.clearVerification()
                    pageStack.pop()
                }
            }
        }
    }
}
