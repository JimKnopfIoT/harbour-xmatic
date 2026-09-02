import QtQuick 2.0
import Sailfish.Silica 1.0

// Interactive verification: agree, then compare seven emoji. Confirming rules
// out a machine in the middle and unlocks shared room keys.
Page {
    id: page

    objectName: "verificationPage"

    allowedOrientations: Orientation.All

    // Whether this side has answered and is waiting. "They match" and "Accept"
    // change nothing visible, and a Silica button colours only under the finger.
    property bool answered: false

    /// The stage this side answered in, so the waiting ends on real progress.
    property string answeredIn: ""

    Connections {
        target: matrix
        // Only a stage the flow actually moved to ends the waiting: the core repeats a
        // stage, and clearing on any notification would let the button go live again.
        onVerificationChanged: {
            if (page.answered && matrix.verificationState !== page.answeredIn) {
                page.answered = false
            }
        }
    }

    // Leaving the page mid-flow would strand the other side waiting.
    onStatusChanged: {
        if (status === PageStatus.Inactive
                && (matrix.verificationState === "done"
                    || matrix.verificationState === "cancelled")) {
            matrix.clearVerification()
        }
    }

    SilicaFlickable {
        id: flickable

        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        // Reported as "there are no buttons": they were below the edge and nothing
        // said the page continued. Seven emoji push the answer off a short screen.
        VerticalScrollDecorator { flickable: flickable }

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingLarge

            // No `textFormat` here: PageHeader has no such property and assigning it kills
            // the page at load time. A user id cannot carry markup anyway.
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
                    // Said before the state is looked at: the flow stays in the same state until
                    // the other side moves, so only this distinguishes waiting from nothing.
                    if (page.answered) {
                        return qsTr("Your answer is in. Waiting for the other device.")
                    }
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
                running: page.answered
                         || (matrix.verificationState === "requested"
                             && (matrix.verificationWeStarted || matrix.encryptionBusy))
            }

            // Stacked, not side by side: a `Row` takes its width from its children and a
            // `WrapButton` from its parent - the loop that left this page with no buttons.
            Column {
                width: parent.width
                spacing: Theme.paddingMedium
                visible: matrix.verificationState === "comparing"
                         || (matrix.verificationState === "requested"
                             && !matrix.verificationWeStarted)

                WrapButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    // Refusing still works while waiting — that is the way out
                    // if the other side never answers.
                    label: matrix.verificationState === "comparing"
                          ? qsTr("They do not match")
                          : qsTr("Decline")
                    onClicked: {
                        // Two meanings, two codes on the wire: mismatched emoji are the one signal
                        // that says somebody may be in the middle. Declining is an ordinary cancel.
                        if (matrix.verificationState === "comparing") {
                            matrix.reportVerificationMismatch()
                        } else {
                            matrix.cancelVerification()
                        }
                        pageStack.pop()
                    }
                }

                WrapButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    enabled: !page.answered
                    label: matrix.verificationState === "comparing"
                          ? qsTr("They match")
                          : qsTr("Accept")
                    onClicked: {
                        page.answeredIn = matrix.verificationState
                        page.answered = true
                        if (matrix.verificationState === "comparing") {
                            matrix.confirmVerification()
                        } else {
                            matrix.acceptVerification()
                        }
                    }
                }
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: matrix.verificationState === "requested"
                         && matrix.verificationWeStarted
                label: qsTr("Cancel")
                onClicked: {
                    matrix.cancelVerification()
                    pageStack.pop()
                }
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: matrix.verificationState === "done"
                         || matrix.verificationState === "cancelled"
                label: qsTr("Close")
                onClicked: {
                    matrix.clearVerification()
                    pageStack.pop()
                }
            }
        }
    }
}
