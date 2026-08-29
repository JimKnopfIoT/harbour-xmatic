import QtQuick 2.0
import Sailfish.Silica 1.0

// What other people may do to this device. The call rules are enforced in the
// core, before an invitation reaches the screen: a refused call rings nothing
// and is answered in no way the caller can observe.
Page {
    id: page

    allowedOrientations: Orientation.All

    // Anything left over from another page is not this page's news.
    onStatusChanged: {
        if (status === PageStatus.Activating) {
            matrix.clearLastError()
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        VerticalScrollDecorator {}

        Column {
            id: content

            width: parent.width
            spacing: Theme.paddingMedium

            // One direction only, and now it is the other one: a wrapping
            // button takes its width as given and lays the label out inside it,
            // so deriving that width from the children's implicitWidth is not
            // just forbidden but meaningless - a WrapButton carries no text of
            // its own, so its implicit width is the platform minimum and every
            // button on the page collapsed to it.
            readonly property real buttonWidth: Math.min(
                    Theme.buttonWidthLarge,
                    width - 2 * Theme.horizontalPageMargin)

            PageHeader {
                title: qsTr("Privacy")
            }

            // What went wrong, where it went wrong. Writing the lists that name
            // people is refused while the store key is unavailable - after a
            // reboot, until the device is unlocked once - and this page had
            // nowhere to say so: an address was entered, nothing appeared, and
            // with "only these may call" that means nobody gets through while
            // the user believes the opposite.
            //
            // The field is the app's only one, so it is emptied when this page
            // opens (below): what stands here afterwards was caused here.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                textFormat: Text.PlainText
                visible: matrix.lastError.length > 0
                text: matrix.lastError
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
                // Says what the switch does and stops there. It shuts this
                // phone's camera and shows no picture; it does not stop the
                // other side from sending one, and their stream is still
                // decoded before it is thrown away. Refusing the video line in
                // the SDP answer is the fix for that, and it belongs with the
                // other call work.
                description: qsTr("Off, an offer with video is answered as a voice call: your camera stays shut and no picture is shown. The other side may still send one, which this phone discards.")
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
                // Says the one direction this switch actually governs. The
                // other one - whether *their* reading is shown here - is the
                // switch below, and claiming both made this one look broken to
                // anybody who left that one on.
                description: qsTr("Off, nobody is told how far you have read. What others read is the setting below.")
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
                // Asked, not claimed. The local storage is encrypted where a
                // key could be minted and unencrypted where it could not, and
                // this page used to state the good case unconditionally - on a
                // device without secrets storage the privacy page said the
                // opposite of the truth, in the very paragraph that is otherwise
                // scrupulous about what is *not* protected.
                text: (matrix.storageStatus.encrypted
                       ? qsTr("Messages and keys are stored encrypted.")
                       : qsTr("Messages and keys lie on this device unencrypted - the encryption page says why and what can be done about it."))
                      + " "
                      + qsTr("Pictures, videos and documents you opened are not - they lie on the device like the ones in the gallery, readable to anybody who has it.")
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

            WrapButton {
                id: clearMediaButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                label: qsTr("Delete media now")
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

            WrapButton {
                id: allowButton

                anchors.horizontalCenter: parent.horizontalCenter
                width: content.buttonWidth
                label: qsTr("Allow calls")
                enabled: addressField.text.trim().length > 0
                onClicked: page.add()
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: matrix.allowedCallers.length === 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: matrix.privateListsReadable ? Theme.secondaryHighlightColor
                                                   : Theme.errorColor
                // An empty list and a list that could not be read look the same
                // from here, and they are opposites: with "only these may call"
                // the second means everybody is refused while the page claims
                // there is nobody to refuse.
                text: matrix.privateListsReadable
                      ? qsTr("Nobody yet.")
                      : qsTr("The list is encrypted and its key is not available. It can be read again after the device has been unlocked and the app restarted.")
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
