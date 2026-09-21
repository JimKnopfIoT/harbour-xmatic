import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0

Page {
    id: page

    property string roomId
    property string roomName

    property bool loaded: false
    property var can: ({})
    property var bridge: null
    property string savedName
    property string savedTopic
    property string avatarUrl

    property int savingText: 0
    property bool savingAvatar: false
    property string errorText
    property bool savedHint: false

    // Acted on when on top again: no push during a pop.
    property string pendingAction

    readonly property bool bridgeUnknown: bridge !== null && bridge.unknown === true
    readonly property bool bridgedGroup: bridge !== null && !bridgeUnknown && bridge.direct !== true
    readonly property bool bridgedDirect: bridge !== null && !bridgeUnknown && bridge.direct === true
    readonly property string network: bridge !== null ? (bridge.network || "") : ""
    readonly property bool mayChangeAny: can.name === true || can.topic === true
                                         || can.avatar === true
    readonly property bool nameChanged: can.name === true
                                        && nameField.text.trim() !== savedName
    readonly property bool topicChanged: can.topic === true
                                         && topicArea.text.trim() !== savedTopic

    readonly property string unknownHint: qsTr("Whether this room is bridged to another network could not be checked. If it is, the change may apply there for everyone in the chat.")

    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.roomSettings.load(roomId)

    onStatusChanged: {
        if (status === PageStatus.Active && pendingAction === "pick") {
            pendingAction = ""
            pageStack.push(avatarPicker)
        }
    }

    function confirmed(action) {
        if (!page.bridgedGroup && !page.bridgeUnknown) {
            action()
            return
        }
        var dialog = pageStack.push(
                    Qt.resolvedUrl("ConfirmDialog.qml"),
                    {
                        question: page.bridgeUnknown
                                  ? qsTr("Change it anyway?")
                                  : qsTr("Change it on the other network as well?"),
                        subject: page.roomName,
                        explanation: page.bridgeUnknown
                                     ? page.unknownHint
                                     : qsTr("The bridge passes the change on. Everyone in the chat there sees it, including people who do not use Matrix."),
                        acceptLabel: qsTr("Change")
                    })
        dialog.accepted.connect(action)
    }

    function saveText() {
        page.errorText = ""
        page.savedHint = false
        if (page.nameChanged) {
            page.savingText++
            matrix.roomSettings.setName(page.roomId, nameField.text)
        }
        if (page.topicChanged) {
            page.savingText++
            matrix.roomSettings.setTopic(page.roomId, topicArea.text)
        }
    }

    function setAvatar(path) {
        page.errorText = ""
        page.savedHint = false
        page.savingAvatar = true
        matrix.roomSettings.setAvatar(page.roomId, path)
    }

    function removeAvatar() {
        page.errorText = ""
        page.savedHint = false
        page.savingAvatar = true
        matrix.roomSettings.removeAvatar(page.roomId)
    }

    Connections {
        target: matrix.roomSettings

        onLoaded: {
            if (settings.roomId !== page.roomId) {
                return
            }
            page.can = settings.can || {}
            page.bridge = settings.bridge || null
            page.savedName = settings.name || ""
            page.savedTopic = settings.topic || ""
            page.avatarUrl = settings.avatar || ""
            nameField.text = page.savedName
            topicArea.text = page.savedTopic
            page.loaded = true
        }

        onSaved: {
            if (roomId !== page.roomId) {
                return
            }
            if (field === "avatar") {
                page.savingAvatar = false
                page.avatarUrl = value
            } else {
                page.savingText = Math.max(0, page.savingText - 1)
                if (field === "name") {
                    page.savedName = value
                    if (!nameField.activeFocus) {
                        nameField.text = value
                    }
                } else {
                    page.savedTopic = value
                    if (!topicArea.activeFocus) {
                        topicArea.text = value
                    }
                }
            }
            if (shownName.length > 0) {
                page.roomName = shownName
            }
            page.savedHint = page.savingText === 0 && !page.savingAvatar
        }

        onFailed: {
            if (roomId !== page.roomId) {
                return
            }
            if (field === "avatar") {
                page.savingAvatar = false
            } else if (field.length > 0) {
                page.savingText = Math.max(0, page.savingText - 1)
            }
            page.savedHint = false
            page.errorText = field === "name" ? qsTr("The name could not be changed: %1").arg(error)
                           : field === "topic" ? qsTr("The topic could not be changed: %1").arg(error)
                           : field === "avatar" ? qsTr("The picture could not be changed: %1").arg(error)
                           : error
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

            readonly property real buttonWidth: Math.min(
                    Theme.buttonWidthLarge,
                    width - 2 * Theme.horizontalPageMargin)

            PageHeader {
                title: qsTr("Edit room")
                description: page.roomName
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Large
                running: !page.loaded
                visible: running
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.loaded
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: !page.mayChangeAny
                      ? qsTr("Your role in this room does not allow changing its name, topic or picture.")
                      : page.bridgeUnknown
                        ? page.unknownHint
                        : page.bridgedGroup
                        ? qsTr("This room is bridged to another network. The bridge usually passes a new name, topic or picture on, and there it changes for everyone in the chat, including people who do not use Matrix.")
                        : page.bridgedDirect
                          ? qsTr("This chat is bridged to another network. There, name and picture belong to the contact: the bridge usually passes nothing on, and it can overwrite a name set here whenever the contact changes theirs. Which contact name the bridge shows is part of its own configuration.")
                          : qsTr("Name, topic and picture belong to the room: everyone in it sees the change.")
            }

            DetailItem {
                visible: page.loaded && page.network.length > 0
                label: qsTr("Bridged to")
                value: page.network
            }

            SectionHeader {
                visible: page.loaded
                text: qsTr("Picture")
            }

            Avatar {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.loaded
                size: Theme.itemSizeExtraLarge
                source: page.avatarUrl
                name: page.roomName
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                visible: page.loaded && page.can.avatar === true
                enabled: !page.savingAvatar
                label: page.avatarUrl.length > 0 ? qsTr("Change picture")
                                                 : qsTr("Set picture")
                onClicked: {
                    if (page.bridgedGroup || page.bridgeUnknown) {
                        page.confirmed(function() { page.pendingAction = "pick" })
                    } else {
                        pageStack.push(avatarPicker)
                    }
                }
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                visible: page.loaded && page.can.avatar === true && page.avatarUrl.length > 0
                enabled: !page.savingAvatar
                label: qsTr("Remove picture")
                onClicked: page.confirmed(page.removeAvatar)
            }

            SectionHeader {
                visible: page.loaded
                text: qsTr("Name and topic")
            }

            TextField {
                id: nameField

                width: parent.width
                visible: page.loaded
                readOnly: page.can.name !== true
                label: qsTr("Name")
                placeholderText: qsTr("No name set")
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: topicArea.focus = true
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.loaded && page.can.name === true && nameField.text.trim().length === 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Without a name, the room is shown under the names of its members.")
            }

            TextArea {
                id: topicArea

                width: parent.width
                visible: page.loaded
                readOnly: page.can.topic !== true
                label: qsTr("Topic")
                placeholderText: qsTr("No topic set")
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                width: column.buttonWidth
                visible: page.loaded && (page.can.name === true || page.can.topic === true)
                enabled: page.savingText === 0 && (page.nameChanged || page.topicChanged)
                label: qsTr("Save")
                onClicked: {
                    nameField.focus = false
                    topicArea.focus = false
                    page.confirmed(page.saveText)
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.savingText > 0 || page.savingAvatar
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Saving…")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.savedHint && page.errorText.length === 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Saved")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: page.errorText.length > 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                text: page.errorText
            }
        }
    }

    Component {
        id: avatarPicker

        // No orientation: the platform's sub-pages would not inherit it.
        ImagePickerPage {
            onSelectedContentPropertiesChanged: {
                if (selectedContentProperties.filePath.length > 0) {
                    page.setAvatar(selectedContentProperties.filePath)
                }
            }
        }
    }
}
