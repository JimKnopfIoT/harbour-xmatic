import QtQuick 2.5
import Sailfish.Silica 1.0
import QtMultimedia 5.6
import "Composing.js" as Composing

// One thread of a room: root first, replies below, own composer. The rows are
// the room's own (MessageRow); this page is their host on the thread's view.
Page {
    id: page

    property string roomId
    property string roomName
    property string rootEventId
    /// Whether the room this thread belongs to is encrypted - a thread is exactly
    /// as encrypted as its room.
    property bool encrypted: false
    /// Until the room has answered, a preview gate reads the thread as encrypted.
    property bool encryptionKnown: false

    /// The room's unverified recipients: a thread reaches the same people, so it
    /// asks the same question before sending.
    property var unverifiedUsers: []

    // Page-local: matrix.lastError is global and would show a stray failure
    // from anywhere as this thread's.
    property string error: ""
    // The error came from a post: its text is back in the field, nothing to retry.
    property bool sendFailed: false
    // The first page came back at the thread's start: empty now means empty.
    property bool reachedStart: false

    // Whether the view follows the newest post. True until the hand says
    // otherwise, exactly as in the room.
    property bool followTail: true

    // The list's height before the last change, for the same reason the room
    // keeps it: what was on screen has to stay on screen.
    property real lastViewHeight: 0

    // What MessageRow asks of its host, answered as the room answers it.
    readonly property bool inThread: true
    /// A mention from the member page lands here when this is the room's topmost page.
    readonly property bool takesMentions: true
    readonly property bool inSlice: false
    readonly property string markerEventId: ""
    readonly property real avatarSize: Theme.iconSizeMedium
    readonly property string messageLayout: settings.messageLayout
    readonly property bool flatLayout: messageLayout === "flat"
    readonly property real systemAvatarSize: Theme.iconSizeExtraSmall
    readonly property var animatableFormats: matrix.animatableImageFormats()
    readonly property bool canRedactOthers: matrix.roomPermissions.redactOthers === true
    property int openMenus: 0
    property var expandedRows: ({})
    property int expandedRevision: 0
    property alias timelineActions: threadActions

    // Set while a post is being rewritten, or answered.
    property string editingEventId: ""
    property string replyingEventId: ""
    property string replyingTo: ""
    /// The quoted text, for the quote on the send page.
    property string replyingBody: ""

    // What the actions page chose, applied once this page is on top again.
    property var pendingAction: null

    property string noticeText: ""

    function systemShown(kind) {
        return threadActions.systemShown(kind)
    }

    function toggleExpanded(rowId) {
        if (page.expandedRows[rowId]) {
            delete page.expandedRows[rowId]
        } else {
            page.expandedRows[rowId] = true
        }
        page.expandedRevision++
    }

    // A quote tapped in the thread: its row, where the thread has it.
    function jumpToEvent(eventId) {
        var index = matrix.threadTimeline.indexOfEvent(eventId || "")
        if (index < 0) {
            return
        }
        page.followTail = false
        threadView.positionViewAtIndex(index, ListView.Center)
    }

    function beginReply(eventId, sender, body) {
        page.replyingEventId = eventId
        page.replyingTo = sender && sender.length > 0 ? sender : body
        page.replyingBody = body || ""
        page.editingEventId = ""
        composer.focusField()
    }

    function cancelReply() {
        page.clearReplyState()
        composer.clearField()
    }

    // After an attachment went out as a reply: the composer stays as it is.
    function clearReplyState() {
        page.replyingEventId = ""
        page.replyingTo = ""
        page.replyingBody = ""
    }

    // The thread again, and its room first where another one is open.
    function reopen() {
        page.error = ""
        page.sendFailed = false
        page.reachedStart = false
        matrix.openThread(page.roomId, page.rootEventId)
    }

    // What failed, again: the open where the thread is not open, else the page.
    function retry() {
        if (!matrix.threadOpen(page.rootEventId)) {
            page.reopen()
            return
        }
        page.error = ""
        page.sendFailed = false
        matrix.threadLoadOlder(page.rootEventId)
    }

    function beginEdit(eventId, body) {
        page.replyingEventId = ""
        page.editingEventId = eventId
        composer.clearMentions()
        composer.text = body
        composer.focusField()
    }

    function cancelEdit() {
        page.editingEventId = ""
        composer.clearField()
        composer.clearMentions()
        composer.releaseField()
    }

    function showNotice(text) {
        page.noticeText = text
        noticeTimer.restart()
    }

    function pickEmoji() {
        var dialog = pageStack.push(Qt.resolvedUrl("ReactionDialog.qml"),
                                    { forMessage: true })
        dialog.accepted.connect(function() {
            if (dialog.key.length > 0) {
                composer.insertAtCursor(dialog.key)
            }
            composer.focusField()
        })
    }

    // Extra work goes inside this handler: a second `onStatusChanged` makes the page unloadable.
    onStatusChanged: {
        if (status !== PageStatus.Active) {
            return
        }
        // A room opened further up (a link) closed this thread.
        if (!matrix.threadOpen(page.rootEventId)) {
            page.reopen()
        }
        if (!pendingAction) {
            return
        }
        var action = pendingAction
        pendingAction = null
        if (action.kind === "reply") {
            beginReply(action.eventId, action.senderName, action.body)
        } else if (action.kind === "edit") {
            beginEdit(action.eventId, action.body)
        } else if (action.kind === "react") {
            threadActions.pickReaction(action.eventId)
        } else if (action.kind === "forwardAttachment") {
            threadActions.forwardAttachment(action.item)
        } else if (action.kind === "transcribe") {
            matrix.transcripts.transcribe(action.item.id, action.eventId, action.item.media)
        } else if (action.kind === "delete") {
            confirmDelete(action.eventId, action.txnId, action.unsent)
        } else if (action.kind === "mention") {
            composer.insertMember(action.userId, action.displayName)
        }
    }

    allowedOrientations: Orientation.All

    Component.onCompleted: {
        matrix.openThread(roomId, rootEventId)
        refreshRecipients()
    }
    Component.onDestruction: matrix.closeThread(page.rootEventId)

    Connections {
        target: matrix
        onThreadFailed: {
            if (root !== page.rootEventId) {
                return
            }
            // No message: an older open's refused post, only its text comes back.
            if (message.length > 0) {
                page.error = message
                // Read after the bridge's own bookkeeping: a closed thread is reopened, not resent.
                page.sendFailed = unsent.length > 0 && matrix.threadOpen(page.rootEventId)
            }
            // A refused post comes back into an empty field rather than being lost.
            if (unsent.length > 0 && composer.text.length === 0) {
                composer.text = unsent
            }
        }
        onThreadPaged: {
            if (root === page.rootEventId) {
                page.reachedStart = atStart
            }
        }
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

    /// Recipients the user has not chosen to trust yet. Empty means nothing
    /// stands in the way of sending.
    function submit() {
        // The uncommitted word, same as in RoomPage.submit() - the reason is
        // written out there.
        Qt.inputMethod.commit()
        var pending = Composing.pendingUnverified(page.unverifiedUsers, matrix)
        if (pending.length > 0) {
            var dialog = pageStack.push(Qt.resolvedUrl("UnverifiedRecipientsDialog.qml"),
                                        { users: pending })
            dialog.accepted.connect(page.doSubmit)
            return
        }
        doSubmit()
    }

    // Leaving the foreground aborts a running countdown, as in the room.
    property var activeRemorse: null
    readonly property bool appForeground: Qt.application.active
    onAppForegroundChanged: {
        if (!appForeground && activeRemorse && activeRemorse.pending) {
            activeRemorse.cancel()
        }
    }

    // The room's countdown, with its guard: Silica runs it when the page goes.
    function confirmDelete(eventId, txnId, unsent) {
        page.activeRemorse = Remorse.popupAction(page, unsent ? qsTr("Discarding") : qsTr("Deleting"), function() {
            if (page.status !== PageStatus.Active || !Qt.application.active) {
                return
            }
            matrix.deleteMessage(eventId, txnId, page.rootEventId)
        })
    }

    function doSubmit() {
        // Closed under the page (another room was opened): the text stays, the thread comes back.
        if (!matrix.threadOpen(page.rootEventId)) {
            page.reopen()
            return
        }
        var mentions = composer.mentionIds()
        // A new post or edit replaces the old one's failure.
        if (page.sendFailed) {
            page.error = ""
            page.sendFailed = false
        }
        if (page.editingEventId.length > 0) {
            matrix.editMessage(page.editingEventId, composer.text, mentions, page.rootEventId)
            page.cancelEdit()
            return
        }
        if (page.replyingEventId.length > 0) {
            matrix.replyToMessage(page.replyingEventId, composer.text, mentions, page.rootEventId)
            page.clearReplyState()
        } else if (!matrix.sendThreadMessage(page.rootEventId, composer.text, mentions)) {
            return
        }
        composer.clearField()
        composer.clearMentions()
        // The same rule as in the room below: with the setting on, the
        // keyboard goes once the post is away.
        page.followTail = true
        if (settings.hideKeyboardOnSend) {
            composer.releaseField()
            return
        }
        composer.holdKeyboard()
        composer.focusField()
    }

    // A tapped link, the room's own handling: `TimelineActions.followLink`.
    function followLink(link, map) {
        threadActions.followLink(link, map)
    }

    // The member page's "Mention", when this is the room's topmost page with a composer.
    Connections {
        target: matrix.mentions

        onInsertRequested: {
            if (roomId !== page.roomId || Composing.mentionHost(pageStack, roomId) !== page) {
                return
            }
            if (page.status === PageStatus.Active) {
                composer.insertMember(userId, displayName)
                return
            }
            page.pendingAction = {
                "kind": "mention",
                "userId": userId,
                "displayName": displayName
            }
            pageStack.pop(page)
        }
    }

    SilicaListView {
        id: threadView

        anchors {
            left: parent.left
            right: parent.right
            top: parent.top
            bottom: composerColumn.top
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

        // Counting rows was not enough: a row is laid out before its text wraps, and
        // each of those grows the content under a view that thought it was at the end.
        onContentHeightChanged: {
            if (page.followTail) {
                tailTimer.restart()
            }
        }

        // The viewport loses height at its bottom while Qt keeps the content's top.
        // Shrinking, push the content down; growing, put a follower back at the end.
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

        // A hand on the list decides where it stays: reading further up must not be
        // undone by the next arriving post.
        onMovementEnded: page.followTail = atYEnd

        Timer {
            id: tailTimer
            interval: 1
            onTriggered: {
                // As in the room: a hand on the list outranks the tail, or the posts a swipe
                // fetched throw the view to the end under the finger.
                if (threadView.moving || threadView.dragging) {
                    return
                }
                threadView.positionViewAtEnd()
            }
        }

        PullDownMenu {
            MenuItem {
                text: qsTranslate("SessionOfflinePage", "Try again")
                visible: page.error.length > 0 && !page.sendFailed
                onClicked: page.retry()
            }

            // At the start it says so instead of offering what is not there.
            MenuItem {
                text: page.reachedStart ? qsTranslate("RoomPage", "Beginning of the conversation")
                                        : qsTr("Load older posts")
                enabled: !page.reachedStart
                onClicked: matrix.threadLoadOlder(page.rootEventId)
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

        delegate: MessageRow {
            host: page
            listView: threadView
            player: threadPlayer
        }

        // Without the error state this would say "loading" for good when the
        // thread cannot be fetched at all.
        ViewPlaceholder {
            enabled: threadView.count === 0
            text: page.error.length > 0 ? qsTr("Thread unavailable")
                                        : page.reachedStart ? qsTr("No posts")
                                                            : qsTr("Loading thread")
        }

        VerticalScrollDecorator { }
    }

    // Sending on the thread timeline threads automatically. The same composer
    // the room below uses, with what is being answered or rewritten above it.
    Column {
        id: composerColumn

        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }

        Row {
            x: Theme.horizontalPageMargin
            width: parent.width - Theme.horizontalPageMargin
            visible: page.replyingEventId.length > 0
            spacing: Theme.paddingSmall

            Label {
                width: parent.width - parent.spacing - cancelReplyButton.width
                anchors.verticalCenter: parent.verticalCenter
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.highlightColor
                truncationMode: TruncationMode.Fade
                text: qsTr("Reply to %1").arg(page.replyingTo)
            }

            IconButton {
                id: cancelReplyButton

                icon.source: "image://theme/icon-m-clear"
                onClicked: page.cancelReply()
            }
        }

        Row {
            x: Theme.horizontalPageMargin
            width: parent.width - Theme.horizontalPageMargin
            visible: page.editingEventId.length > 0
            spacing: Theme.paddingSmall

            Label {
                width: parent.width - parent.spacing - cancelEditButton.width
                anchors.verticalCenter: parent.verticalCenter
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.highlightColor
                truncationMode: TruncationMode.Fade
                text: qsTr("Editing message")
            }

            IconButton {
                id: cancelEditButton

                icon.source: "image://theme/icon-m-clear"
                onClicked: page.cancelEdit()
            }
        }

        Composer {
            id: composer

            anchors {
                left: parent.left
                right: parent.right
            }

            roomId: page.roomId
            attachments: true
            emoji: true
            editing: page.editingEventId.length > 0
            placeholderText: page.editingEventId.length > 0 ? qsTr("New text")
                                                            : qsTr("Reply in thread")
            onSubmitted: page.submit()
            onAttachRequested: threadAttachments.pick()
            onEmojiRequested: page.pickEmoji()
        }
    }

    // The thread's own player; `TimelineActions.play` stops the room's.
    Audio {
        id: threadPlayer

        onStopped: {
            if (status === Audio.EndOfMedia) {
                threadActions.playingId = ""
            }
        }
    }

    // Media, reactions, names and shields on the thread's own view and media keys.
    TimelineActions {
        id: threadActions

        page: page
        player: threadPlayer
        threadRoot: page.rootEventId
        mediaPrefix: "thread:"
        timelineModel: matrix.threadTimeline
    }

    Rectangle {
        id: notice

        anchors {
            horizontalCenter: parent.horizontalCenter
            bottom: composerColumn.top
            bottomMargin: Theme.paddingLarge
        }
        width: noticeLabel.width + 2 * Theme.paddingLarge
        height: noticeLabel.height + 2 * Theme.paddingMedium
        radius: Theme.paddingMedium
        color: Theme.rgba(Theme.highlightDimmerColor, 0.9)
        opacity: 0

        Label {
            id: noticeLabel

            anchors.centerIn: parent
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.primaryColor
            text: page.noticeText
        }

        Behavior on opacity { FadeAnimation { } }
    }

    Timer {
        id: noticeTimer

        interval: 2500
        onTriggered: notice.opacity = 0
        onRunningChanged: {
            if (running) {
                notice.opacity = 1
            }
        }
    }

    ThreadAttachments {
        id: threadAttachments

        page: page
        composer: composer
        roomId: page.roomId
        threadRoot: page.rootEventId
        unverifiedUsers: page.unverifiedUsers
    }
}
