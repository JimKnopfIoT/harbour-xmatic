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
    /// Whether the room this thread belongs to is encrypted. Everything the
    /// warning below rests on hangs off it, and a thread is exactly as
    /// encrypted as its room.
    property bool encrypted: false

    /// Recipients of that room whose devices are unverified, as the room view
    /// holds them. A thread reaches the same people as the room does, so it
    /// asks the same question before sending - it used to reach them without
    /// asking anything.
    property var unverifiedUsers: []

    // Page-local: matrix.lastError is global and would show a stray failure
    // from anywhere as this thread's.
    property string error: ""

    // Whether the view follows the newest post. True until the hand says
    // otherwise, exactly as in the room.
    property bool followTail: true

    // The list's height before the last change, for the same reason the room
    // keeps it: what was on screen has to stay on screen.
    property real lastViewHeight: 0

    allowedOrientations: Orientation.All

    Component.onCompleted: {
        matrix.openThread(roomId, rootEventId)
        refreshRecipients()
    }
    Component.onDestruction: matrix.closeThread()

    Connections {
        target: matrix
        onThreadFailed: page.error = message
        onRecipientsChecked: {
            if (roomId === page.roomId) {
                page.unverifiedUsers = users
            }
        }
    }

    function refreshRecipients() {
        if (page.encrypted) {
            matrix.checkRecipients(page.roomId)
        }
    }

    /// The one line a message carries when its authenticity is in doubt. Same
    /// codes and same wording as the room view: a thread is part of the same
    /// conversation and must not say less about a message than the room does.
    function shieldText(shield) {
        if (!shield) {
            return ""
        }
        switch (shield.reason) {
        case "sentInClear": return qsTr("Sent unencrypted")
        case "mismatchedSender": return qsTr("Not sent by the account it names")
        case "identityChanged": return qsTr("The sender's keys changed")
        case "unsignedDevice":
        case "unknownDevice": return qsTr("From an unverified device")
        case "unverifiedIdentity": return qsTr("From an unverified person")
        default: return qsTr("Authenticity not confirmed")
        }
    }

    /// Recipients the user has not chosen to trust yet. Empty means nothing
    /// stands in the way of sending.
    function pendingUnverified() {
        var out = []
        for (var i = 0; i < unverifiedUsers.length; i++) {
            var entry = unverifiedUsers[i]
            if (!matrix.recipientTrusted(entry.userId)) {
                out.push(entry)
            }
        }
        return out
    }

    function submit() {
        var pending = pendingUnverified()
        if (pending.length > 0) {
            var dialog = pageStack.push(Qt.resolvedUrl("UnverifiedRecipientsDialog.qml"),
                                        { users: pending })
            dialog.accepted.connect(page.doSubmit)
            return
        }
        doSubmit()
    }

    function doSubmit() {
        matrix.sendThreadMessage(threadInput.text)
        threadInput.text = ""
        // The same rule as in the room below: with the setting on, the
        // keyboard goes once the post is away.
        page.followTail = true
        if (settings.hideKeyboardOnSend) {
            threadInput.focus = false
        }
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
        onCountChanged: {
            if (page.followTail) {
                tailTimer.restart()
            }
        }

        // Counting rows was not enough, which is what "it does not properly go
        // to the bottom" was: a row is laid out before its text has wrapped,
        // the page header settles a moment later, and every one of those grows
        // the content under a view that already thought it was at the end.
        // Both belong in this one handler - a second onContentHeightChanged in
        // the same object is a load error, not an addition.
        onContentHeightChanged: {
            if (page.followTail) {
                tailTimer.restart()
            }
        }

        // The viewport's own height changes under the list - the keyboard
        // takes the lower half of the screen, the composer grows with what is
        // being typed - and Qt keeps the content's top where it is, so what
        // was being read slides out of sight below. Shrinking, the content is
        // pushed down by exactly what was lost; growing, whoever was following
        // the tail goes back to the end. The room does the same.
        onHeightChanged: {
            var lost = page.lastViewHeight - height
            page.lastViewHeight = height
            if (lost > 0) {
                contentY = Math.min(contentY + lost,
                                    originY + Math.max(0, contentHeight - height))
            } else if (lost < 0 && page.followTail) {
                tailTimer.restart()
            }
        }

        // A hand on the list decides where it stays - reading further up must
        // not be undone by the next arriving post, and pulling down for older
        // posts must not throw the view back to the end.
        onMovementEnded: page.followTail = atYEnd

        Timer {
            id: tailTimer
            interval: 1
            onTriggered: {
                // As in the room: a hand on the list outranks the tail, or
                // the posts fetched by a swipe throw the view to the end
                // under the finger that asked for them.
                if (threadView.moving || threadView.dragging) {
                    return
                }
                threadView.positionViewAtEnd()
            }
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

                // What the SDK will not vouch for. Red is a message that is
                // not what it claims to be, grey one it cannot check; silent
                // otherwise.
                Label {
                    width: parent.width
                    visible: !!model.shield
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: model.shield && model.shield.level === "red"
                           ? Theme.errorColor : Theme.secondaryColor
                    textFormat: Text.PlainText
                    text: page.shieldText(model.shield)
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
            onClicked: page.submit()
        }
    }
}
