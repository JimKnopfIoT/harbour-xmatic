import QtQuick 2.0
import Sailfish.Silica 1.0

// The account's push rules per kind of message: off, on, or with a tone. Kept on
// the server, so every client of the account follows the same levels.
Page {
    id: page

    allowedOrientations: Orientation.All

    property string error: ""
    // `read`, `write` or `late`, as `failed` names it.
    property string failure: ""

    readonly property var levelNames: ["off", "on", "noisy"]

    Component.onCompleted: matrix.pushRules.load()

    Connections {
        target: matrix.pushRules
        onLevelsChanged: page.error = ""
        onFailed: {
            page.error = error
            page.failure = kind
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height + Theme.paddingLarge

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: qsTr("Notification rules")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Stored in your account, so every client follows them. With tone rings, Without tone shows a notification and blinks the LED, Off only raises the room's unread count; a mention set to Off counts as an ordinary message of its room. A room with its own setting in its details keeps it.")
            }

            Row {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                spacing: Theme.paddingMedium
                visible: matrix.pushRules.busy

                BusyIndicator {
                    id: waiting
                    size: BusyIndicatorSize.ExtraSmall
                    running: matrix.pushRules.busy
                }

                Label {
                    width: parent.width - waiting.width - parent.spacing
                    anchors.verticalCenter: waiting.verticalCenter
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.secondaryHighlightColor
                    text: qsTr("Waiting for the server…")
                }
            }

            Repeater {
                model: [
                    { "key": "direct", "label": qsTr("Direct messages") },
                    { "key": "group", "label": qsTr("Group messages") },
                    { "key": "mention", "label": qsTr("Mentions of you") },
                    { "key": "room", "label": qsTr("Mentions of the whole room") },
                    { "key": "invite", "label": qsTr("Invitations") },
                    { "key": "call", "label": qsTr("Calls") }
                ]

                ComboBox {
                    id: combo

                    readonly property string key: modelData.key

                    function shown() {
                        return page.levelNames.indexOf(matrix.pushRules.levels[key] || "")
                    }

                    // A kind the server has no rule for is not offered.
                    visible: matrix.pushRules.loaded && matrix.pushRules.levels[key] !== undefined
                    enabled: !matrix.pushRules.busy
                    label: modelData.label
                    // The rules of this kind disagree, so no level is selected.
                    description: matrix.pushRules.levels[key] === "mixed"
                                 ? qsTr("Set differently in parts. Pick a level to apply it throughout.")
                                 : ""
                    currentIndex: shown()

                    // A pick breaks the binding; the answer, or a refusal, puts it back.
                    Connections {
                        target: matrix.pushRules
                        onLevelsChanged: combo.currentIndex = combo.shown()
                    }

                    menu: ContextMenu {
                        MenuItem {
                            text: qsTr("Off")
                            onClicked: matrix.pushRules.set(combo.key, "off")
                        }
                        MenuItem {
                            text: qsTr("Without tone")
                            onClicked: matrix.pushRules.set(combo.key, "on")
                        }
                        MenuItem {
                            text: qsTr("With tone")
                            onClicked: matrix.pushRules.set(combo.key, "noisy")
                        }
                    }
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.error.length > 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                text: page.failure === "late"
                      ? qsTr("The server did not answer in time.")
                      : page.failure === "write"
                        ? qsTr("The server refused the change: %1").arg(page.error)
                        : qsTr("Could not read the rules from the server: %1").arg(page.error)
            }

            // Levels not read back are the last known ones, or none at all.
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: (!matrix.pushRules.loaded || matrix.pushRules.stale) && !matrix.pushRules.busy
                text: qsTranslate("SessionOfflinePage", "Try again")
                onClicked: matrix.pushRules.load()
            }
        }
    }
}
