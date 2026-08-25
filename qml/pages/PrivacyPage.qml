import QtQuick 2.0
import Sailfish.Silica 1.0

// What other people may do to this device. The call rules are enforced in the
// core, before an invitation reaches the screen: a refused call rings nothing
// and is answered in no way the caller can observe.
Page {
    id: page

    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            // One width for the buttons on this page - see AccountPage.
            readonly property real buttonWidth: Math.min(
                    width - 2 * Theme.horizontalPageMargin,
                    Math.max(clearMediaButton.implicitWidth, allowButton.implicitWidth))

            PageHeader {
                title: qsTr("Privacy")
            }

            ComboBox {
                width: parent.width
                label: qsTr("Who may call you")
                description: qsTr("A call rings past a muted room. Refused calls never ring, and the caller learns nothing.")
                currentIndex: settings.callPolicy === "all"
                              ? 0 : (settings.callPolicy === "direct" ? 1 : 2)

                menu: ContextMenu {
                    MenuItem { text: qsTr("Everyone") }
                    MenuItem { text: qsTr("People you have a direct chat with") }
                    MenuItem { text: qsTr("Only my list") }
                }

                onCurrentIndexChanged: {
                    settings.callPolicy = currentIndex === 0
                            ? "all" : (currentIndex === 1 ? "direct" : "list")
                }
            }

            TextSwitch {
                text: qsTr("Calls from group rooms")
                description: qsTr("In a group room everybody sees the call. The list does not override this.")
                checked: settings.groupCalls
                automaticCheck: false
                onClicked: settings.groupCalls = !settings.groupCalls
            }

            TextSwitch {
                text: qsTr("Limit repeated calls")
                description: qsTr("On, the same person can only ring again after a short pause. It also delays a second, genuine attempt.")
                checked: settings.callFlood
                automaticCheck: false
                onClicked: settings.callFlood = !settings.callFlood
            }

            TextSwitch {
                text: qsTr("Video calls")
                description: qsTr("Off, an offer with video is answered as a voice call.")
                checked: settings.videoCalls
                automaticCheck: false
                onClicked: settings.videoCalls = !settings.videoCalls
            }

            SectionHeader {
                text: qsTr("What others learn")
            }

            TextSwitch {
                text: qsTr("Message text in notifications")
                description: qsTr("Off, a notification says only how many messages arrived. On, it shows the latest message — also on the lock screen.")
                checked: settings.notificationPreview
                automaticCheck: false
                onClicked: settings.notificationPreview = !settings.notificationPreview
            }

            TextSwitch {
                text: qsTr("Send read receipts")
                description: qsTr("Off keeps your reading to yourself, in both directions.")
                checked: settings.sendReadReceipts
                automaticCheck: false
                onClicked: settings.sendReadReceipts = !settings.sendReadReceipts
            }

            TextSwitch {
                text: qsTr("Show others' read status")
                description: qsTr("Off, nothing is fetched about who read what, which also keeps the conversation smoother. On, your own messages say how many people have read them.")
                checked: settings.showReadStatus
                automaticCheck: false
                onClicked: settings.showReadStatus = !settings.showReadStatus
            }

            TextSwitch {
                text: qsTr("Voice messages")
                description: qsTr("On, a microphone sits next to the message field: hold it to record, let go to send. Off, it is not there.")
                checked: settings.voiceMessages
                automaticCheck: false
                onClicked: settings.voiceMessages = !settings.voiceMessages
            }

            TextSwitch {
                text: qsTr("Tappable web links")
                description: qsTr("On, a link in a message opens the browser when tapped. Off, links stay plain text.")
                checked: settings.clickableLinks
                automaticCheck: false
                onClicked: settings.clickableLinks = !settings.clickableLinks
            }

            SectionHeader {
                text: qsTr("On this device")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                // Both halves are this app's own text, so the markup is ours;
                // the second one is bold because it is the one that costs
                // something when it is overlooked.
                textFormat: Text.StyledText
                text: qsTr("Messages and keys are stored encrypted. Pictures, videos and documents you opened are not - they lie on the device like the ones in the gallery, readable to anybody who has it.")
                      + " <b>" + qsTr("By default they are deleted when you sign out, so save what you want to keep.") + "</b> "
                      + qsTr("\"Never\" keeps them for good - convenient, and not recommended.")
            }

            ComboBox {
                width: parent.width
                label: qsTr("Delete downloaded media")
                description: qsTr("Anything deleted is fetched again when you open it.")
                currentIndex: {
                    switch (settings.mediaWipe) {
                    case "never": return 0
                    case "exit": return 2
                    case "background": return 3
                    default: return 1
                    }
                }

                menu: ContextMenu {
                    MenuItem { text: qsTr("Never") }
                    MenuItem { text: qsTr("When you sign out") }
                    MenuItem { text: qsTr("When the app is closed") }
                    MenuItem { text: qsTr("As soon as the app is not in front") }
                }

                onCurrentIndexChanged: {
                    switch (currentIndex) {
                    case 0: settings.mediaWipe = "never"; break
                    case 2: settings.mediaWipe = "exit"; break
                    case 3: settings.mediaWipe = "background"; break
                    default: settings.mediaWipe = "logout"; break
                    }
                }
            }

            Button {
                id: clearMediaButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                text: qsTr("Delete media now")
                onClicked: matrix.clearMediaCache()
            }

            SectionHeader {
                text: qsTr("Allowed callers")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("They may always call. The list stays on this device.")
            }

            TextField {
                id: addressField

                width: parent.width
                label: qsTr("Matrix address")
                placeholderText: qsTr("@name:server")
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.add()
            }

            Button {
                id: allowButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                text: qsTr("Allow calls")
                enabled: addressField.text.trim().length > 0
                onClicked: page.add()
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: matrix.allowedCallers.length === 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Nobody yet.")
            }

            Repeater {
                model: matrix.allowedCallers

                ListItem {
                    contentHeight: Theme.itemSizeSmall

                    menu: ContextMenu {
                        MenuItem {
                            text: qsTr("Remove")
                            onClicked: matrix.forbidCaller(modelData)
                        }
                    }

                    Label {
                        anchors {
                            left: parent.left
                            leftMargin: Theme.horizontalPageMargin
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin
                            verticalCenter: parent.verticalCenter
                        }
                        truncationMode: TruncationMode.Fade
                        textFormat: Text.PlainText
                        text: modelData
                    }
                }
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }
    }

    function add() {
        var address = addressField.text.trim()
        if (address.length === 0) {
            return
        }
        matrix.allowCaller(address)
        addressField.text = ""
        addressField.focus = false
    }
}
