import QtQuick 2.0
import Sailfish.Silica 1.0
import "Formatting.js" as Formatting
import "MatrixLinks.js" as MatrixLinks

// One thread of a room: root first, replies below, own composer. Rows stream
// into matrix.threadTimeline via `thread.diff`. Text-focused: attachments
// render as their kind, undecryptable and redacted rows stay visible.
Page {
    id: page

    property string roomId
    property string roomName
    property string rootEventId

    // Page-local: matrix.lastError is global and would show a stray failure
    // from anywhere as this thread's.
    property string error: ""

    allowedOrientations: Orientation.All

    Component.onCompleted: matrix.openThread(roomId, rootEventId)
    Component.onDestruction: matrix.closeThread()

    Connections {
        target: matrix
        onThreadFailed: page.error = message
    }

    // A tapped link, as in the room below: a Matrix address is answered inside
    // the app, anything else is a web address.
    function followLink(link) {
        var target = MatrixLinks.parse(link)
        if (!target) {
            // The same allowlist as the room view: everything the system would
            // otherwise dispatch is a stranger's choice, not the user's.
            if (/^https?:\/\//i.test(link)) {
                Qt.openUrlExternally(link)
            }
            return
        }
        if (target.kind === "user") {
            pageStack.push(Qt.resolvedUrl("NewChatDialog.qml"), { prefill: target.id })
            return
        }
        page.pendingAddress = target.id
        matrix.resolveRoom(target.id)
    }

    property string pendingAddress: ""

    Connections {
        target: matrix

        onRoomResolved: {
            if (page.pendingAddress.length === 0) {
                return
            }
            page.pendingAddress = ""
            if (joined) {
                pageStack.push(Qt.resolvedUrl("RoomPage.qml"),
                               { roomId: roomId, roomName: "" })
                return
            }
            pageStack.push(Qt.resolvedUrl("JoinRoomDialog.qml"), { prefill: address })
        }

        onRoomResolveFailed: page.pendingAddress = ""
    }

    SilicaListView {
        id: threadView

        anchors {
            left: parent.left
            right: parent.right
            top: parent.top
            bottom: composer.top
        }
        clip: true
        model: matrix.threadTimeline
        // Row 0 must not be made current: a current item inside a view's
        // focus scope takes the focus from the composer on every reset.
        currentIndex: -1

        // New posts arrive at the end; without this they land outside the
        // view. The timer defers to after the row exists, as in RoomPage.
        onCountChanged: tailTimer.restart()

        Timer {
            id: tailTimer
            interval: 1
            onTriggered: threadView.positionViewAtEnd()
        }

        PullDownMenu {
            MenuItem {
                text: qsTr("Load older posts")
                onClicked: matrix.threadLoadOlder()
            }
        }

        header: Column {
            width: threadView.width

            PageHeader {
                title: qsTr("Thread")
                description: page.roomName
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                textFormat: Text.PlainText
                visible: page.error.length > 0
                text: page.error
            }
        }

        delegate: Item {
            id: threadRow

            // "system" covers calls, membership and profile changes; without
            // a row of its own a thread made only of those would render as an
            // empty page while the model is full.
            readonly property bool isSystem: model.kind === "system"

            width: threadView.width
            height: {
                if (model.kind === "date") {
                    return dateLabel.height + Theme.paddingMedium
                }
                if (threadRow.isSystem) {
                    return systemLabel.height + Theme.paddingMedium
                }
                if (rowColumn.visible) {
                    return rowColumn.height + Theme.paddingMedium
                }
                return 0
            }

            Label {
                id: systemLabel

                visible: threadRow.isSystem
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                font.italic: true
                color: Theme.secondaryColor
                textFormat: Text.PlainText
                text: threadRow.isSystem
                      ? (model.name ? model.name + " · " : "") + qsTr("Event")
                      : ""
            }

            Label {
                id: dateLabel

                visible: model.kind === "date"
                anchors.horizontalCenter: parent.horizontalCenter
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: model.kind === "date"
                      ? Format.formatDate(new Date(model.timestamp), Formatter.DateMedium)
                      : ""
            }

            Column {
                id: rowColumn

                visible: model.kind === "message"
                         || model.kind === "undecryptable"
                         || model.kind === "redacted"
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                spacing: Theme.paddingSmall / 2

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.highlightColor
                    textFormat: Text.PlainText
                    text: (model.senderName || model.sender || "")
                          + " · "
                          + Format.formatDate(new Date(model.timestamp), Formatter.TimeValue)
                }

                Label {
                    width: parent.width
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    font.italic: model.kind !== "message"
                    color: model.kind === "message" ? Theme.primaryColor
                                                    : Theme.secondaryColor
                    // Same rule as in the room: a message whose HTML the core
                    // turned into markup is drawn as StyledText, everything
                    // else stays plain. Nothing in that markup came from the
                    // sender unescaped.
                    readonly property bool hasFormatted: model.kind === "message"
                                                         && !model.media
                                                         && (model.formatted || "").length > 0
                    textFormat: hasFormatted ? Text.StyledText : Text.PlainText
                    linkColor: Theme.highlightColor
                    onLinkActivated: page.followLink(link)
                    text: {
                        if (model.kind === "undecryptable") {
                            return qsTr("Cannot be decrypted — this device is missing the key")
                        }
                        if (model.kind === "redacted") {
                            return qsTr("Message deleted")
                        }
                        if (model.media) {
                            return "📎 " + (model.body || qsTr("Attachment"))
                        }
                        if (hasFormatted) {
                            return Formatting.renderFormatted(model.formatted,
                                                              settings.clickableLinks)
                        }
                        return model.body || ""
                    }
                }
            }
        }

        // Without the error state this would say "loading" for good when the
        // thread cannot be fetched at all.
        ViewPlaceholder {
            enabled: threadView.count === 0
            text: page.error.length > 0 ? qsTr("Thread unavailable")
                                        : qsTr("Loading thread")
        }

        VerticalScrollDecorator { }
    }

    // Sending on the thread timeline threads automatically. Laid out like the
    // room's composer: no label, so focusing does not jump the height.
    Row {
        id: composer

        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }

        TextArea {
            id: threadInput

            width: parent.width - sendButton.width
            placeholderText: qsTr("Reply in thread")
            labelVisible: false
        }

        IconButton {
            id: sendButton

            anchors.bottom: threadInput.bottom
            anchors.bottomMargin: Theme.paddingMedium
            width: Theme.itemSizeSmall
            icon.source: "image://theme/icon-m-send"
            enabled: threadInput.text.trim().length > 0
            onClicked: {
                matrix.sendThreadMessage(threadInput.text)
                threadInput.text = ""
            }
        }
    }
}
