import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import QtMultimedia 5.6
import "Formatting.js" as Formatting
import "MatrixLinks.js" as MatrixLinks
import "SecurityStatus.js" as SecurityStatus

// A single room: history above, composer below. The list keeps the core's
// order and follows the bottom unless the user has scrolled up.
Page {
    id: page

    // Named so the window can see whether the room a notification is about is
    // already the page on screen.
    objectName: "roomPage"

    property string roomId
    property string roomName
    property bool invited: false
    // Defaults to true so push sites that do not know the state never offer
    // enabling encryption where it may already be on.
    property bool encrypted: true
    // Whether the encryption value came from the room itself. Half the ways in
    // carry none, and a security mark whose failure state is "safe" is worse than none.
    property bool encryptionKnown: false

    /// How many message menus stand open - at most one, but counted rather than
    /// flagged so a delegate that dies with its menu cannot leave it set.
    property int openMenus: 0

    /// The open came back as a failure, with the core's scrubbed reason. The
    /// placeholder says so instead of leaving a spinner over nothing.
    property bool openFailed: false
    property string openFailedReason: ""

    // Media keys whose fetch failed - refused, dropped, given up on. Without them
    // the row cannot tell "still loading" from "never coming".
    property var failedMedia: ({})

    // Set while an already sent message is being rewritten.
    property string editingEventId: ""
    // Set while a reply is being composed.
    property string replyingEventId: ""
    /// The quoted message's text, for the quote on the send page.
    property string replyingBody: ""
    property string replyingTo: ""

    allowedOrientations: Orientation.All

    // The sender's picture and the gap it leaves. One number for picture, bubble
    // margin and text width - never read off the item, which is the width rule.
    readonly property real avatarSize: Theme.iconSizeMedium

    // Whether new messages should scroll the view along.
    property bool followTail: true

    // The timeline's height before the last change: keyboard, banner and composer
    // all take height away, and what was on screen has to stay there.
    property real lastTimelineHeight: 0

    // A save that is waiting for its download to finish.
    property string pendingSaveKey: ""
    /// Forwarding an attachment needs the file on this device first - the target
    /// room encrypts under its own keys, and a thumbnail is not the file.
    property string pendingForwardKey: ""
    property string pendingForwardMime: ""

    // The audio item currently loaded into the player, and one waiting for its
    // download.
    property string playingId: ""
    property string pendingPlayKey: ""
    property bool pendingSaveIsImage: false
    property string pendingSaveName: ""

    // Set from the pinned overview: once the live timeline is back, scroll to
    // this message — in this normal window, not in a separate view.
    property string jumpTargetId: ""
    // How many pages of history a jump may still fetch before giving up. An
    // old pin can sit far beyond what is loaded.
    property int jumpPagesLeft: 0
    // How many turns it may spend waiting for the live timeline to be there at
    // all - roughly ten seconds, after which the room is plainly not coming.
    property int jumpWaitsLeft: 0
    readonly property int jumpWaitLimit: 15

    // Recipients with unverified devices, from `checkRecipients`. Drives the
    // pre-send warning; each entry is { userId, name, devices }.
    property var unverifiedUsers: []

    // Set while the join into the room that replaced this one is in flight.
    // Its own flag, not `busy`: that one is true for any command at all.
    property bool followingSuccessor: false

    // What the actions page chose, applied once this page is on top again: focus
    // set while the stack animates does not stick, and a push during a pop is lost.
    property var pendingAction: null

    // Fires before the children go, so the field can still be read. An edit is not
    // a draft, and an empty field removes what stood there before.
    Component.onDestruction: {
        if (!invited) {
            // Same reason as in submit(): without this the draft keeps
            // everything except the word the user stopped in the middle of.
            Qt.inputMethod.commit()
            matrix.setDraft(roomId,
                            page.editingEventId.length > 0 ? "" : messageField.text)
        }
    }

    // Leaving the foreground aborts a running countdown at once: suppressing it at
    // expiry left it firing on return from a minimised app.
    property var activeRemorse: null
    readonly property bool appForeground: Qt.application.active
    onAppForegroundChanged: {
        if (!appForeground && activeRemorse && activeRemorse.pending) {
            activeRemorse.cancel()
        }
    }


    Component.onCompleted: {
        if (!invited) {
            // What was typed here last time. Going back destroys this page, so the field
            // is filled from somewhere that outlives it.
            messageField.text = matrix.draft(roomId)
            matrix.openRoom(roomId)
            refreshRecipients(true)
            // The lock has to state the room's answer, not the caller's guess - not every
            // way in carries one, and the property defaults to encrypted.
            matrix.loadRoomInfo(roomId)
        }
    }

    // When the recipients were last asked about, so a return to this page does
    // not ask again straight away.
    property double lastRecipientCheck: 0
    readonly property int recipientCheckInterval: 60000

    // Encrypted rooms only, and throttled: this walks every member and asks the
    // server about unknown devices. Devices do not change by the minute.
    function refreshRecipients(force) {
        if (!page.encrypted) {
            return
        }
        var now = new Date().getTime()
        if (!force && page.lastRecipientCheck > 0
                && now - page.lastRecipientCheck < page.recipientCheckInterval) {
            return
        }
        page.lastRecipientCheck = now
        matrix.checkRecipients(page.roomId)
    }

    // Where reading stopped last time, until the row it names is on screen.
    property string unreadFromId: ""

    // The line's own event, frozen at entry. The SDK's marker row moves the moment
    // this visit's receipt goes out and would end up under the newest message.
    property string markerEventId: ""

    // The second guess: a marker can name an event this room has no row for.
    // Cleared as it is used, so the search can never become a loop.
    property string unreadFallbackId: ""

    // How many pages the search may still ask for. The first batch is the newest
    // slice, and the more is unread the further back the marker sits.
    property int unreadPagesLeft: 0
    readonly property int unreadPageLimit: 8
    /// How many rounds the search may wait for the first rows before it starts
    /// paginating. At the retry timer's 700 ms that is about seven seconds.
    property int unreadEmptyRounds: 0
    readonly property int unreadEmptyLimit: 10

    // Once per built timeline: coming back from a sub-page must not jump. A
    // rebuilt timeline is a different matter - the rows are new.
    property bool unreadHandled: false

    Connections {
        target: matrix

        onMediaFailed: {
            var marked = page.failedMedia
            marked[key] = true
            // Reassigned, not mutated in place: a JavaScript object changed
            // through its own reference tells no binding anything.
            page.failedMedia = marked
        }

        // What the room says about itself against what the caller believed: encryption,
        // and the name where a notification tap arrived with the id alone.
        onRoomInfoReady: {
            if (info.roomId === page.roomId) {
                page.encrypted = info.encrypted === true
                page.encryptionKnown = true
                if (page.roomName.length === 0 && info.name.length > 0) {
                    page.roomName = info.name
                }
            }
        }

        onTimelineOpened: {
            // The live timeline is back, which is what a pending jump waited for. It goes
            // first: the message the user asked for beats where reading stopped.
            if (page.jumpTargetId.length > 0) {
                page.tryJump()
            }
            if (rebuilt) {
                page.unreadHandled = false
            }
            if (page.unreadHandled) {
                return
            }
            page.unreadHandled = true
            page.markerEventId = readMarker
            page.unreadFallbackId = readReceipt
            // The line is drawn either way; only the opening position is a choice. Off,
            // the room stays at its newest message and the line is met by scrolling.
            if (!settings.jumpToReadMarker) {
                return
            }
            page.unreadFromId = readMarker
            page.unreadPagesLeft = page.unreadPageLimit
            page.unreadEmptyRounds = page.unreadEmptyLimit
            // The tail is let go before the search starts: a view that follows the newest
            // row marks the room read, which moves the marker being looked for.
            if (readMarker.length > 0) {
                page.followTail = false
            }
            unreadTimer.restart()
        }
    }

    // Positioning right after a model change lands on the old geometry.
    Timer {
        id: unreadTimer

        interval: 1
        onTriggered: page.showFirstUnread()
    }

    // Throttled so a burst of rows does not produce one command per row. The SDK
    // drops a receipt already covered, so a late one costs nothing.
    Timer {
        id: readTimer

        interval: 800
        onTriggered: {
            // Only from the tail, and `atYEnd` counts as much as the flag - a jump or an
            // unread open switches it off with the newest message on screen.
            if (page.unreadFromId.length > 0) {
                return
            }
            if ((page.followTail || timelineView.atYEnd)
                    && page.status === PageStatus.Active
                    && Qt.application.active) {
                matrix.markRead()
            }
        }
    }

    // Not while the marker search runs: a room opens at its end, and marking read
    // there moves the very marker it is looking for.
    Connections {
        target: Qt.application
        onActiveChanged: {
            if (page.status !== PageStatus.Active || page.invited) {
                return
            }
            if (Qt.application.active) {
                // Back in front: whether this counts as reading is decided by
                // the same test as everywhere else, one timer tick later.
                readTimer.restart()
            } else {
                page.markReadIfDue()
            }
        }
    }

    // Leaving is the last moment to say the room was read, and it has to be taken
    // on `Deactivating` - a popped page never reaches `Inactive`.
    function markReadIfDue() {
        if (invited || page.unreadFromId.length > 0) {
            return
        }
        if (page.followTail || timelineView.atYEnd) {
            matrix.markRead()
        }
    }

    // Opens where reading stopped, with the last read message at the top so the
    // line and the first unread are both in view. Bounded pagination.
    function showFirstUnread() {
        var marker = page.unreadFromId
        if (marker.length === 0 || page.jumpTargetId.length > 0) {
            page.unreadFromId = ""
            unreadRetry.stop()
            return
        }
        var idx = matrix.timeline.indexOfEvent(marker)
        if (idx >= 0) {
            page.unreadFromId = ""
            unreadRetry.stop()
            // The marker on the newest row means everything is read: nothing
            // to jump to, and the room belongs at its end.
            if (idx >= matrix.timeline.count - 1) {
                console.warn("xmatic: read marker is the newest row ("
                             + idx + " of " + matrix.timeline.count
                             + "), staying at the end")
                page.stayAtEnd()
                return
            }
            console.warn("xmatic: opening at the read marker, row " + idx
                         + " of " + matrix.timeline.count)
            timelineView.positionViewAtIndex(idx, ListView.Beginning)
            // What "at the end" means is only known once the view settled: two unread
            // messages fit, a longer run does not, and without asking the room parked.
            followCheck.restart()
            return
        }
        // An empty model here means "not yet": the core answers `timeline.open` before
        // the rows arrive. Bounded, so a room that never delivers cannot hold the view.
        if (matrix.timeline.count === 0 && page.unreadEmptyRounds > 0) {
            page.unreadEmptyRounds--
            unreadRetry.restart()
            return
        }
        if (page.unreadPagesLeft > 0 && !matrix.timelineAtStart) {
            // One request at a time; the timer brings us back. On a timer, not on the row
            // count: a page can arrive full of events that render as nothing.
            if (!matrix.paginating) {
                page.unreadPagesLeft--
                matrix.loadOlder()
            }
            unreadRetry.restart()
            return
        }
        // The beginning is loaded and the row is still missing, so the marker names no
        // row here. The receipt is tried once, on the pagination budget that is left.
        if (page.unreadFallbackId.length > 0 && page.unreadFallbackId !== marker) {
            console.warn("xmatic: read marker is not a row in this room, "
                         + "falling back to the read receipt")
            page.unreadFromId = page.unreadFallbackId
            page.markerEventId = page.unreadFallbackId
            page.unreadFallbackId = ""
            unreadRetry.restart()
            return
        }
        // Out of reach: the room opens at its newest message, which is where
        // it stood before this looked.
        console.warn("xmatic: read marker not found, "
                     + matrix.timeline.count + " rows, "
                     + page.unreadPagesLeft + " pages left, at start "
                     + matrix.timelineAtStart + " - staying at the end")
        page.unreadFromId = ""
        unreadRetry.stop()
        page.stayAtEnd()
    }

    // The end, and everything that goes with being there: the newest rows are
    // followed again and the room counts as read.
    function stayAtEnd() {
        page.followTail = true
        timelineView.positionViewAtEnd()
        readTimer.restart()
    }

    Timer {
        id: unreadRetry

        interval: 700
        onTriggered: page.showFirstUnread()
    }

    Timer {
        id: followCheck

        interval: 50
        onTriggered: {
            page.followTail = timelineView.atYEnd
            if (page.followTail) {
                readTimer.restart()
            }
        }
    }

    Connections {
        target: matrix
        // Verifying somebody is the moment the warning should stop naming them - the
        // throttle would keep the old answer for a minute.
        onVerificationChanged: page.refreshRecipients(true)
        onTimelineFailed: {
            page.openFailed = true
            page.openFailedReason = reason
        }
        // Any open that gets under way clears the last failure: the flag
        // belongs to one attempt, not to the page.
        onOpenRoomChanged: page.openFailed = false

        onRecipientsChecked: {
            if (roomId === page.roomId) {
                page.unverifiedUsers = users
            }
        }
    }

    // Deliberately not closed on leaving: the kept subscription makes stepping
    // back instant. The core keeps one, and opening another room replaces it.

    onStatusChanged: {
        // Which room is on screen, which is not the open room: the core keeps that
        // one's timeline subscribed after the page is gone.
        if (status === PageStatus.Active) {
            matrix.setVisibleRoom(roomId)
        } else if (status === PageStatus.Deactivating
                   || status === PageStatus.Inactive) {
            matrix.setVisibleRoom("")
            page.markReadIfDue()
        }

        if (status === PageStatus.Active && !invited) {
            // Coming back from the pinned view, the shared timeline has to show live
            // events again. On a live timeline this is a no-op.
            matrix.openRoom(roomId)
            // Not marked read outright: the view may open at the first unread message, and
            // everything below it is unread until the user gets there.
            readTimer.restart()
            tryJump()
            // The room's state again: encryption can have been switched on one page
            // further in, and the lock would still stand open.
            matrix.loadRoomInfo(roomId)
            // Devices may have been verified (or new ones appeared) while the
            // page was covered; re-check so the warning stays current.
            refreshRecipients()

            // Extra work goes inside this handler: QML refuses a type that binds
            // `onStatusChanged` twice, and the page then fails to load.
            if (pendingAction) {
                var action = pendingAction
                pendingAction = null
                if (action.kind === "reply") {
                    beginReply(action.eventId, action.senderName, action.body)
                } else if (action.kind === "edit") {
                    beginEdit(action.eventId, action.body)
                } else if (action.kind === "sendMedia") {
                    submitMedia(action)
                } else if (action.kind === "react") {
                    // These three push a page of their own. From the actions page, after its pop
                    // had started, `push()` handed nothing back and the picker never appeared.
                    pickReaction(action.eventId)
                } else if (action.kind === "thread") {
                    openThread(action.eventId)
                } else if (action.kind === "forwardAttachment") {
                    forwardAttachment(action.item)
                } else if (action.kind === "delete") {
                    // The countdown belongs on the page that stays: started on the actions page,
                    // Silica executes it the moment that page pops.
                    confirmDelete(action.eventId, action.txnId, action.unsent)
                }
            }
        }
    }

    // Leaving is irreversible and used to be one tug away: a dialog that names the
    // room, then the remorse as the undo.
    function confirmLeave() {
        var dialog = pageStack.push(
                    Qt.resolvedUrl("ConfirmDialog.qml"),
                    {
                        question: page.invited ? qsTr("Really decline this invitation?")
                                               : qsTr("Really leave this room?"),
                        subject: page.roomName,
                        explanation: page.invited
                                     ? qsTr("The invitation is gone afterwards. You can only get back in if somebody invites you again.")
                                     : qsTr("The room is left and forgotten. It disappears from the chat list, and getting back in needs a new invitation or a public address."),
                        acceptLabel: page.invited ? qsTr("Decline") : qsTr("Leave")
                    })
        dialog.accepted.connect(function() { page.startLeave() })
    }

    function startLeave() {
        page.activeRemorse = Remorse.popupAction(
                    page,
                    page.invited ? qsTr("Declining") : qsTr("Leaving room"),
                    function() {
                        // Silica executes a running remorse the moment its page deactivates, so going
                        // back completed it. Only a countdown on its own page, app in front, counts.
                        if (page.status !== PageStatus.Active || !Qt.application.active) {
                            return
                        }
                        matrix.leaveRoom(page.roomId)
                        if (pageStack.currentPage === page) {
                            pageStack.pop()
                        }
                    })
    }

    // Deleting is not undoable and used to be one tap away. The countdown runs on
    // this page; a failed send is discarded rather than deleted.
    function confirmDelete(eventId, txnId, unsent) {
        page.activeRemorse = Remorse.popupAction(
                    page,
                    unsent ? qsTr("Discarding") : qsTr("Deleting"),
                    function() {
                        // The same test the room's own remorse makes: Silica executes a countdown when
                        // its page deactivates, and minimising leaves it running unseen.
                        if (page.status !== PageStatus.Active || !Qt.application.active) {
                            return
                        }
                        matrix.deleteMessage(eventId || "", txnId || "")
                    })
    }

    // What the row says instead of "not decryptable": withheld key or older than
    // this device is what decides whether anything can be done about it.
    function undecryptableText(cause) {
        switch (cause) {
        case "withheldInsecure":
            return qsTr("The sender did not share the key: they consider this device insecure. Verify this device.")
        case "withheldBySender":
            return qsTr("The sender could not deliver the key to this device.")
        case "verificationViolation":
            return qsTr("The sender's identity has changed since you verified them, so the key was withheld.")
        case "unsignedDevice":
            return qsTr("The sender's device is not signed by its owner.")
        case "unknownDevice":
            return qsTr("The sender's device is unknown here.")
        case "sentBeforeJoin":
            return qsTr("Sent before you joined the room.")
        case "historicalNoBackup":
            return qsTr("Older than this device, and there is no key backup.")
        case "historicalUnverifiedDevice":
            return qsTr("Older than this device. Verify this device to read it.")
        default:
            return qsTr("Cannot be decrypted — this device is missing the key")
        }
    }

    // Scrolls to a message, loading older history until it shows up. Called by
    // the pinned overview before it pops back, and by tapping a quote.
    function jumpToEvent(eventId) {
        if (!eventId || eventId.length === 0) {
            return
        }
        jumpTargetId = eventId
        jumpPagesLeft = 10
        jumpWaitsLeft = jumpWaitLimit
        // Only when this page is on screen: the pinned overview calls this while the
        // shared model still holds the pinned view, and the switch back throws it away.
        if (page.status === PageStatus.Active) {
            tryJump()
        }
    }

    // Kept under its old name for the pinned overview, which calls it on the
    // page underneath.
    function jumpToPinned(eventId) {
        jumpToEvent(eventId)
    }

    function tryJump() {
        if (jumpTargetId.length === 0) {
            return
        }
        // Nothing to look in yet - coming back from the pinned view empties the model.
        // Bounded, so a room that never delivers stops asking.
        if (!matrix.timelineReady || matrix.timeline.count === 0) {
            if (jumpWaitsLeft > 0) {
                jumpWaitsLeft--
                jumpRetry.restart()
                return
            }
            jumpTargetId = ""
            jumpRetry.stop()
            return
        }
        var idx = matrix.timeline.indexOfEvent(jumpTargetId)
        if (idx >= 0) {
            jumpTargetId = ""
            jumpRetry.stop()
            followTail = false
            timelineView.positionViewAtIndex(idx, ListView.Center)
            return
        }
        if (jumpPagesLeft > 0 && !matrix.timelineAtStart) {
            // Only one request at a time; the timer brings us back either way.
            if (!matrix.paginating) {
                jumpPagesLeft--
                matrix.loadOlder()
            }
            // Retried on a timer, not on the row count: a page of history can arrive full
            // of events that render as nothing, and the jump would die without a word.
            jumpRetry.restart()
            return
        }
        jumpTargetId = ""
        jumpRetry.stop()
        showNotice(qsTr("That message is not in the loaded history"))
    }

    Timer {
        id: jumpRetry

        interval: 700
        onTriggered: page.tryJump()
    }

    // A list shorter than the screen cannot be scrolled, so nothing triggers older
    // messages. Bounded: rows that render as nothing look like growth.
    property int fillEmptyRounds: 0
    readonly property int fillEmptyLimit: 3
    /// Height, not row count: the loop runs while the content is shorter than the
    /// screen, and a pure profile change raises the count by no pixel.
    property real fillLastHeight: -1

    function fillScreen() {
        if (page.invited || !matrix.timelineReady || matrix.paginating
                || matrix.timelineAtStart || timelineView.count === 0) {
            return
        }
        if (timelineView.contentHeight > timelineView.height) {
            return
        }

        // Whether a round brought anything is decided here, not when its reply
        // arrived: the rows come through the diff stream, regularly later.
        if (page.fillLastHeight >= 0
                && timelineView.contentHeight <= page.fillLastHeight) {
            if (++page.fillEmptyRounds >= page.fillEmptyLimit) {
                return
            }
        } else {
            page.fillEmptyRounds = 0
        }

        page.fillLastHeight = timelineView.contentHeight
        matrix.loadOlder()
    }

    Connections {
        target: matrix

        // The round is over, and where it brought nothing no diff will arrive to
        // trigger the fill. Not `busyChanged`: a downloading avatar drove history.
        onPaginated: page.fillScreen()

        // A different room, a fresh start.
        onTimelineReadyChanged: {
            page.fillEmptyRounds = 0
            page.fillLastHeight = -1
            page.fillScreen()
        }
    }

    Connections {
        target: matrix
        onMediaReady: {
            if (key === page.pendingPlayKey) {
                page.pendingPlayKey = ""
                audioPlayer.source = "file://" + path
                audioPlayer.play()
                return
            }
            if (key === page.pendingForwardKey) {
                page.pendingForwardKey = ""
                pageStack.push(Qt.resolvedUrl("ForwardPage.qml"), {
                                   path: "file://" + path,
                                   mimeType: page.pendingForwardMime
                               })
                return
            }
            if (key !== page.pendingSaveKey) {
                return
            }
            page.pendingSaveKey = ""
            if (page.pendingSaveIsImage) {
                matrix.saveToPictures(path, page.pendingSaveName)
            } else {
                matrix.saveToDownloads(path, page.pendingSaveName)
            }
        }
    }

    // The switch into the replacement room is over. A failed join has to clear the
    // wait too, or the banner keeps announcing a move that is not happening.
    Connections {
        target: matrix
        onSuccessorReady: page.followingSuccessor = false
        onLastErrorChanged: page.followingSuccessor = false
    }

    // Unlocking the backup makes keys available that this room was missing.
    Connections {
        target: matrix
        onEncryptionChanged: {
            if (!page.invited && matrix.encryptionStatus.backupEnabled) {
                matrix.fetchRoomKeys(page.roomId)
            }
        }
    }

    // The room's menu hangs on a page-sized flickable, not on the timeline whose
    // top is arbitrarily far away. Dragging the strip opens it, the list scrolls.
    SilicaFlickable {
        id: roomView

        anchors.fill: parent
        contentHeight: height
        contentWidth: width
        // Back to rest whenever nothing holds this flickable: Silica activates a
        // pulley only from `contentY == 0`, and its own snap-back skips a one-screen
        onContentYChanged: {
            if (contentY > 0) {
                contentY = 0
            }
            settleTimer.restart()
        }

        Timer {
            id: settleTimer
            interval: 500
            onTriggered: {
                if (roomView.contentY === 0) {
                    return
                }
                // Re-armed, not dropped: a resting finger stops changing
                // contentY, so nothing would wind this up again.
                if (roomView.dragging || roomView.moving || roomMenu.active) {
                    settleTimer.restart()
                    return
                }
                roomView.contentY = 0
            }
        }

        // Conditions belong on the entries, never on the menu: hiding the pull-down
        // takes every other entry with it and the page has no way out.
        PullDownMenu {
            id: roomMenu

            // Reverse of reading order: the *last* entry is the bottom one, where a short
            // tug lands. Leaving sits at the top, so it takes a full pull.
            MenuItem {
                // Deliberately without a visibility condition: an invitation has to be
                // refusable and a joined room leavable, so this is the way out either way.
                text: page.invited ? qsTr("Decline invitation") : qsTr("Leave room")
                onClicked: page.confirmLeave()
            }

            MenuItem {
                text: qsTr("Call")
                visible: !page.invited && matrix.calls.available
                         && matrix.calls.state === "idle"
                onClicked: {
                    matrix.calls.placeCall(page.roomId)
                    pageStack.push(Qt.resolvedUrl("CallPage.qml"))
                }
            }

            // A call that is still running must always be reachable, or a
            // stuck state leaves the menu looking broken.
            MenuItem {
                text: qsTr("Video call")
                visible: !page.invited && matrix.calls.available
                         && matrix.calls.state === "idle"
                onClicked: {
                    matrix.calls.placeCall(page.roomId, true)
                    pageStack.push(Qt.resolvedUrl("CallPage.qml"))
                }
            }

            MenuItem {
                text: qsTr("Back to the call")
                visible: matrix.calls.state !== "idle"
                onClicked: pageStack.push(Qt.resolvedUrl("CallPage.qml"))
            }

            // The banner is the obvious way, but a menu entry is the one users look for -
            // the banner sits above a conversation scrolled to its end.
            MenuItem {
                text: matrix.replacementJoined ? qsTr("Go to the new room")
                                               : qsTr("Join the new room")
                visible: !page.invited && matrix.roomReplaced
                onClicked: {
                    page.followingSuccessor = true
                    matrix.followSuccessor(page.roomId)
                }
            }

            // The link others need to be pointed at this room. Copied
            // rather than shown: it is only ever wanted somewhere else.
            MenuItem {
                text: qsTr("Copy room link")
                visible: !page.invited
                onClicked: matrix.loadRoomLink(page.roomId)
            }

            // Only in a two-party encrypted chat: the core answers with the other
            // person's address, so nothing has to be typed and groups never show it.
            MenuItem {
                text: qsTr("Verify contact")
                visible: !page.invited && matrix.roomDirectPeer.length > 0
                onClicked: {
                    matrix.requestVerification(matrix.roomDirectPeer)
                    pageStack.push(Qt.resolvedUrl("VerificationPage.qml"))
                }
            }

            // Everything about the room rather than the conversation lives one page
            // further in - this menu had grown to ten entries.
            MenuItem {
                text: qsTr("Room info")
                onClicked: pageStack.push(Qt.resolvedUrl("RoomInfoPage.qml"), {
                                              roomId: page.roomId,
                                              roomName: page.roomName,
                                              invited: page.invited
                                          })
            }

            // Kept here rather than moved: searching is about the running conversation.
            // Above the entry below, so the shortest tug still lands where it did.
            MenuItem {
                text: qsTr("Search messages")
                visible: !page.invited
                onClicked: pageStack.push(Qt.resolvedUrl("SearchPage.qml"), {
                                              roomId: page.roomId,
                                              roomName: page.roomName
                                          })
            }

            // Last, so the shortest tug reaches it: reading further back is what one
            // reaches for in a conversation; the entries above are about leaving it.
            MenuItem {
                text: qsTr("Load older messages")
                visible: !page.invited && !matrix.timelineAtStart
                // Only a running pagination disables this. The global busy flag greyed it out
                // while some unrelated thumbnail was downloading.
                enabled: !matrix.paginating
                onClicked: matrix.loadOlder()
            }
        }

        // The room's name stays put instead of scrolling away with the list,
        // which is what makes the menu above reachable at all.
        BackgroundItem {
            id: roomHeader

            anchors {
                left: parent.left
                right: parent.right
                top: parent.top
                // Out from under the camera cutout: this strip is the room's own, and Silica
                // gives only its own headers that margin. Portrait only, as Silica has it.
                topMargin: page.orientation === Orientation.Portrait
                           ? Screen.topCutout.height : 0
            }
            height: headerColumn.height + 2 * Theme.paddingMedium
            // The room's name leads to what the room is - including, for an invitation,
            // the topic that decides whether to accept.
            onClicked: pageStack.push(Qt.resolvedUrl("RoomInfoPage.qml"), {
                                          roomId: page.roomId,
                                          roomName: page.roomName,
                                          invited: page.invited
                                      })

            Column {
                id: headerColumn

                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }

                // The lock leads the name: closed and green for an encrypted room, open and
                // red without. A shape is looked at; a sentence was read once.
                Row {
                    anchors.right: parent.right
                    spacing: Theme.paddingLarge

                    LockIcon {
                        id: headerLock

                        // Against the Row, not the name beside it: a positioner sets its children's
                        // y, and two of them reaching into each other is a fight nobody needs.
                        anchors.verticalCenter: parent.verticalCenter
                        // An invitation says nothing reliable about encryption, and neither does a
                        // room that has not answered: both claim nothing rather than the wrong thing.
                        visible: !page.invited && page.encryptionKnown
                        size: Math.round(Theme.fontSizeLarge * 1.2)
                        locked: page.encrypted
                        // Faint in both states: the name is what the strip is for. Set by eye on the
                        // device - half was still too present.
                        opacity: 0.30
                    }

                    Label {
                        id: nameLabel

                        // The name takes what the lock leaves. Reading the column's width is safe: it
                        // comes from the strip's anchors, never from inside this Row.
                        width: Math.min(implicitWidth,
                                        headerColumn.width
                                        - (headerLock.visible
                                           ? headerLock.width + parent.spacing : 0))
                        horizontalAlignment: Text.AlignRight
                        text: page.roomName
                        font.pixelSize: Theme.fontSizeLarge
                        font.family: Theme.fontFamilyHeading
                        color: roomHeader.highlighted ? Theme.highlightColor
                                                      : Theme.primaryColor
                        truncationMode: TruncationMode.Fade
                        // A room name with a newline in it would push the strip
                        // open and take the space the conversation needs.
                        wrapMode: Text.NoWrap
                        maximumLineCount: 1
                    }
                }

                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignRight
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: roomHeader.highlighted ? Theme.secondaryHighlightColor
                                                  : Theme.secondaryColor
                    truncationMode: TruncationMode.Fade
                    wrapMode: Text.NoWrap
                    maximumLineCount: 1
                    // Only what the lock cannot say. Being cut off outranks anything about the
                    // room: a silently stale conversation is what the user cannot see.
                    visible: text.length > 0
                    text: matrix.syncState === "offline"
                          ? qsTr("Offline — waiting for the network")
                          : page.invited ? qsTr("Invitation") : ""
                }
            }
        }

        // An upgraded room takes no new messages - the upgrade raises the send level -
        // so this says so and leads on. Above the pinned banner: the rest is history.
        BackgroundItem {
            id: tombstoneBanner

            anchors {
                left: parent.left
                right: parent.right
                top: roomHeader.bottom
            }
            height: visible ? Theme.itemSizeSmall : 0
            visible: !page.invited && matrix.roomReplaced
            clip: true
            onClicked: {
                page.followingSuccessor = true
                matrix.followSuccessor(page.roomId)
            }

            Rectangle {
                anchors.fill: parent
                z: -1
                color: Theme.rgba(Theme.errorColor, 0.15)
            }

            Column {
                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    wrapMode: Text.NoWrap
                    maximumLineCount: 1
                    font.pixelSize: Theme.fontSizeSmall
                    color: tombstoneBanner.highlighted ? Theme.highlightColor
                                                       : Theme.primaryColor
                    text: qsTr("This room has been replaced")
                }

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    wrapMode: Text.NoWrap
                    maximumLineCount: 1
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: tombstoneBanner.highlighted ? Theme.secondaryHighlightColor
                                                       : Theme.secondaryColor
                    // The reason is free text from whoever upgraded and may say nothing useful,
                    // so the way out comes first.
                    text: page.followingSuccessor
                          ? qsTr("Switching to the new room…")
                          : (matrix.replacementJoined
                             ? qsTr("Tap to open the new room")
                             : qsTr("Tap to join the new room"))
                            + (matrix.replacementReason.length > 0
                               ? " · " + matrix.replacementReason : "")
                }
            }
        }

        // Pinned messages sit under the room's name while the conversation scrolls
        // underneath: the banner is outside the list, so it never moves.
        BackgroundItem {
            id: pinnedBanner

            anchors {
                left: parent.left
                right: parent.right
                top: tombstoneBanner.bottom
            }
            height: visible ? Theme.itemSizeExtraSmall : 0
            visible: !page.invited && matrix.pinnedEventIds.length > 0
            // One line high and outside the list: a preview that grew taller would paint
            // over the conversation instead of being cut off.
            clip: true
            onClicked: pageStack.push(Qt.resolvedUrl("PinnedMessagesPage.qml"), {
                                          roomId: page.roomId,
                                          roomName: page.roomName
                                      })

            Rectangle {
                anchors.fill: parent
                z: -1
                color: Theme.rgba(Theme.highlightBackgroundColor, 0.2)
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
                // Fading alone only shortens a line; a body with newlines still
                // breaks into as many lines as it likes and the Label grows with it.
                wrapMode: Text.NoWrap
                maximumLineCount: 1
                font.pixelSize: Theme.fontSizeSmall
                color: pinnedBanner.highlighted ? Theme.highlightColor
                                                : Theme.secondaryHighlightColor
                // The newest pin's text; the count is the fallback while it loads or where the
                // body cannot be read, and a prefix when there are more.
                text: "📌 "
                      + (matrix.pinnedEventIds.length > 1
                         ? "(" + matrix.pinnedEventIds.length + ") " : "")
                      + (matrix.pinnedPreview.length > 0
                         ? matrix.pinnedPreview
                         : qsTr("%n pinned message(s)", "", matrix.pinnedEventIds.length))
            }
        }

        SilicaListView {
            id: timelineView

            anchors {
                left: parent.left
                right: parent.right
                top: pinnedBanner.bottom
                bottom: composer.top
            }
            clip: true
            model: matrix.timeline
            cacheBuffer: page.height

            // Air under the last message: the read mark's tap area reaches a finger's
            // width below the bubble, which is exactly where the message field begins.
            footer: Item {
                width: 1
                height: Math.round(Theme.fontSizeTiny * 1.7)
                        + 2 * Theme.paddingMedium
            }

            header: Column {
                width: timelineView.width

                // Nothing but breathing room: the room's name is the fixed
                // strip above, so the first message must not start glued to it.
                Item {
                    width: 1
                    height: Theme.paddingLarge
                }

                // Without this, "load older messages" simply does nothing once the history is
                // complete, which reads like a broken button.
                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryColor
                    visible: matrix.timelineAtStart && timelineView.count > 0
                    text: qsTr("Beginning of the conversation")
                }
            }

            delegate: ListItem {
                id: row

                // Undecryptable and deleted events get a bubble too, so a gap in
                // the conversation is visible rather than silently skipped.
                readonly property bool isBubble: model.kind === "message"
                                                 || model.kind === "undecryptable"
                                                 || model.kind === "redacted"
                // The last message read before this visit carries the line. Not the SDK's
                // marker row: that needs receipt tracking and it moves as the room is read.
                readonly property bool showMarker: rowEventId.length > 0
                                                   && rowEventId === page.markerEventId
                                                   && index < timelineView.count - 1
                // The text an attachment was sent with. Empty for one sent
                // without, where `body` holds the file name instead.
                readonly property string rowEventId: model.eventId || ""
                readonly property var reactionList: model.reactions || []
                readonly property bool hasReactions: reactionList.length > 0
                // Six fit next to each other on the narrow device; the rest are
                // a count.
                readonly property var reactionsShown: reactionList.slice(0, 6)
                readonly property int reactionsHidden: Math.max(0, reactionList.length - 6)
                readonly property string captionText: model.caption || ""
                readonly property bool hasCaption: captionText.length > 0
                readonly property bool isImage: model.kind === "message"
                                                && model.msgtype === "m.image"
                                                && !!model.media
                readonly property bool isAudio: model.kind === "message"
                                                && model.msgtype === "m.audio"
                                                && !!model.media
                readonly property bool isVideo: model.kind === "message"
                                                && model.msgtype === "m.video"
                                                && !!model.media
                readonly property bool isFile: model.kind === "message"
                                               && !!model.media
                                               && !isImage
                readonly property bool isOwn: model.own === true
                // Only a body that visibly carries a link pays the rich-text path, behind a
                // setting. Never for a file row: its caption is a stranger's text.
                readonly property bool hasLink: model.kind === "message"
                                                && !model.media
                                                && ((settings.clickableLinks
                                                     && /https?:\/\//.test(model.body || ""))
                                                    || MatrixLinks.hasAddress(model.body))
                // The core made this markup itself and escaped the sender's characters, so it
                // can go to StyledText like a linkified body.
                readonly property bool hasFormatted: model.kind === "message"
                                                     && !model.media
                                                     && (model.formatted || "").length > 0

                // The body as markup, or empty where plain text will do. One property for
                // format and text, so a Label can never parse what was not built for it.
                readonly property int emojiPixels: Math.round(Theme.fontSizeSmall * 1.3)
                readonly property string richBody: {
                    if (model.kind !== "message" || !!model.media) {
                        return ""
                    }
                    var base = ""
                    if (hasFormatted) {
                        base = Formatting.renderFormatted(model.formatted,
                                                          settings.clickableLinks)
                    } else if (hasLink) {
                        base = page.linkifyBody(model.body || "")
                    } else if (settings.emojiImages) {
                        base = page.escapeBody(model.body || "")
                    } else {
                        return ""
                    }
                    if (!settings.emojiImages) {
                        return base
                    }
                    var pictured = Formatting.withEmojiPictures(
                                base, emojiPixels,
                                function(key) { return matrix.emojiSource(key) })
                    // A plain body with no picture in it stays plain text -
                    // the markup renderer costs more and buys nothing.
                    if (!hasFormatted && !hasLink && pictured === base) {
                        return ""
                    }
                    return pictured
                }

                // Calls and membership changes show as a centred line so a room made of them
                // is not empty. Pure profile changes collapse but stay, or indices drift.
                readonly property bool isSystem: model.kind === "system"
                                                 && model.system !== "profile"

                // Localised here, not in the core: the token comes from Rust, the
                // wording and language belong to the UI.
                function systemText() {
                    var who = (model.name && model.name.length > 0)
                              ? model.name : model.senderName
                    switch (model.system) {
                    case "call":            return qsTr("Call", "timeline system line, a noun")
                    case "member.joined":   return qsTr("%1 joined").arg(who)
                    case "member.left":     return qsTr("%1 left").arg(who)
                    case "member.invited":  return qsTr("%1 was invited").arg(who)
                    case "member.kicked":   return qsTr("%1 was removed").arg(who)
                    case "member.banned":   return qsTr("%1 was banned").arg(who)
                    case "member.declined": return qsTr("%1 declined the invitation").arg(who)
                    case "member.knocked":  return qsTr("%1 asked to join").arg(who)
                    case "member":          return qsTr("%1 changed membership").arg(who)
                    case "profile":         return qsTr("%1 changed their profile").arg(who)
                    default:                return ""
                    }
                }

                // What the sender declares about a picture. `sourceSize` is a hint an
                // interlaced PNG may ignore, so absurd dimensions are not loaded at all.
                readonly property bool saneDimensions: !model.media
                        || ((model.media.width || 0) <= 8192
                            && (model.media.height || 0) <= 8192)

                readonly property bool sanePicture: !model.media
                        || (saneDimensions
                            // The declared size, before anything is fetched. Nothing is required of it: a
                            // missing size is the common case, and `media::fetch` weighs what arrives.
                            && (model.media.size || 0) <= 100 * 1024 * 1024)

                readonly property bool hasPreview: sanePicture
                                                   && (isImage
                                                       || (isVideo && !!model.media.thumbnailSource))

                width: timelineView.width
                contentHeight: (isBubble
                                ? Math.max(bubble.height,
                                           row.isOwn ? 0 : page.avatarSize)
                                  + Theme.paddingMedium
                                : (model.kind === "date"
                                   ? dayLabel.height + Theme.paddingLarge
                                   : (isSystem
                                      ? systemLabel.height + Theme.paddingLarge
                                      : 0)))
                               // The gap the line sits in. Any row can carry it, so it is added to the row's
                               // own height rather than being a row of its own.
                               + (showMarker ? Theme.paddingLarge : 0)

                // Only real messages react; dividers are not something to press.
                enabled: model.kind === "message"
                _showPress: false

                // A growing height under an open menu is the menu, not new rows:
                // following it scrolls the pressed row off the top.
                onMenuOpenChanged: page.openMenus = Math.max(0, page.openMenus
                                                             + (menuOpen ? 1 : -1))
                // A recycled delegate takes its menu with it, and that closing
                // does not always come back as a `menuOpen` change.
                Component.onDestruction: {
                    if (menuOpen) {
                        page.openMenus = Math.max(0, page.openMenus - 1)
                    }
                }

                menu: ContextMenu {
                    MenuItem {
                        text: qsTr("Copy")
                        visible: (model.body || "").length > 0
                        onClicked: Clipboard.text = model.body
                    }

                    MenuItem {
                        text: qsTr("Reply")
                        visible: model.kind === "message"
                        onClicked: page.beginReply(model.eventId, model.senderName, model.body)
                    }

                    // `!page.isLandscape` below: there the conversation keeps under 600 px, four
                    // menu rows, and a message's menu needs six. Those move to a page.
                    MenuItem {
                        // Starting one, not only answering in one that exists: the marker opens a
                        // thread that is already there.
                        text: qsTr("Reply in thread")
                        visible: model.kind === "message"
                                 && (model.eventId || "").length > 0
                                 && !page.isLandscape
                        onClicked: pageStack.push(Qt.resolvedUrl("ThreadPage.qml"), {
                                                      roomId: page.roomId,
                                                      roomName: page.roomName,
                                                      rootEventId: model.threadRoot
                                                                   && model.threadRoot.length > 0
                                                                   ? model.threadRoot
                                                                   : model.eventId,
                                                      encrypted: page.encrypted
                                                  })
                    }

                    MenuItem {
                        text: qsTr("Save")
                        visible: (row.isFile || row.isImage) && !page.isLandscape
                        onClicked: page.saveAttachment(model)
                    }

                    MenuItem {
                        text: qsTr("Forward")
                        visible: (model.body || "").length > 0 && !row.isImage
                                 && !page.isLandscape
                        onClicked: pageStack.push(Qt.resolvedUrl("ForwardPage.qml"), {
                                                      body: model.body
                                                  })
                    }

                    MenuItem {
                        text: qsTr("Edit")
                        visible: row.isOwn && model.editable === true && !page.isLandscape
                        onClicked: page.beginEdit(model.eventId, model.body)
                    }

                    MenuItem {
                        text: qsTr("Pin")
                        // Only where this account may write the pinned list. `!== false`: until the
                        // room answers there is no entry, and a slow answer must not remove an action.
                        visible: model.kind === "message" && (model.eventId || "").length > 0
                                 && !page.isLandscape
                                 && matrix.roomPermissions.pin !== false
                        onClicked: matrix.pinMessage(model.eventId, true)
                    }

                    MenuItem {
                        text: qsTr("React")
                        visible: model.kind === "message"
                                 && (model.eventId || "").length > 0
                                 && !page.isLandscape
                        onClicked: page.pickReaction(model.eventId)
                    }

                    MenuItem {
                        // The queue gave up on this one; this puts it back in
                        // line. Nothing else in the app could move it.
                        text: qsTr("Send again")
                        visible: model.sendState === "failed" && !page.isLandscape
                        onClicked: matrix.retryMessage(model.txnId || "")
                    }

                    MenuItem {
                        // Also the only way out for a send that failed for good: it has no event id,
                        // and without this it sits in the room for ever.
                        text: model.sendState === "failed"
                              ? qsTr("Discard") : qsTr("Delete")
                        visible: row.isOwn && !page.isLandscape
                        onClicked: page.confirmDelete(model.eventId || "",
                                                      model.txnId || "",
                                                      model.sendState === "failed")
                    }

                    MenuItem {
                        text: qsTr("More…")
                        visible: page.isLandscape
                        onClicked: pageStack.push(Qt.resolvedUrl("MessageActionsPage.qml"), {
                                                      roomPage: page,
                                                      eventId: model.eventId || "",
                                                      txnId: model.txnId || "",
                                                      unsent: model.sendState === "failed",
                                                      body: model.body || "",
                                                      senderName: model.senderName || "",
                                                      isOwn: row.isOwn,
                                                      editable: model.editable === true,
                                                      isImage: row.isImage,
                                                      canSave: row.isFile || row.isImage,
                                                      // Copied out, not handed over: the model row is gone once it leaves the
                                                      // cache.
                                                      item: {
                                                          "id": model.id,
                                                          "msgtype": model.msgtype,
                                                          "media": model.media
                                                      }
                                                  })
                    }
                }

                // The sender's picture, for other people only. Keyed by the address, so five
                // senders in two hundred messages cost five downloads.
                Avatar {
                    id: senderAvatar

                    visible: row.isBubble && !row.isOwn
                    anchors {
                        left: parent.left
                        leftMargin: Theme.horizontalPageMargin
                        top: parent.top
                    }
                    size: page.avatarSize
                    source: model.senderAvatar || ""
                    name: model.senderName || model.sender || ""

                    // Tap opens the sender's profile. The margin stays inside the gap to the
                    // bubble, or it takes taps meant for the message.
                    MouseArea {
                        anchors {
                            fill: parent
                            margins: -Theme.paddingSmall
                        }
                        onClicked: pageStack.push(
                                       Qt.resolvedUrl("MemberProfilePage.qml"),
                                       { roomId: page.roomId, userId: model.sender })
                        onPressAndHold: row.openMenu()
                    }
                }

                // A message. Own messages sit on the right in the highlight colour,
                // everyone else's on the left — the usual reading cue.
                Rectangle {
                    id: bubble

                    visible: row.isBubble
                    anchors {
                        right: row.isOwn ? parent.right : undefined
                        left: row.isOwn ? undefined : parent.left
                        rightMargin: Theme.horizontalPageMargin
                        // A constant, never the avatar's width: reading a sibling's width here and
                        // its height there is how this delegate earns a binding loop.
                        leftMargin: Theme.horizontalPageMargin
                                    + page.avatarSize + Theme.paddingSmall
                        top: parent.top
                    }
                    // Width follows the column, which sizes itself from the texts. Deriving it
                    // from the labels would close the loop - their width comes from the bubble.
                    width: bubbleColumn.width + 2 * Theme.paddingMedium
                    height: bubbleColumn.height + 2 * Theme.paddingMedium
                    radius: Theme.paddingMedium
                    // Both fills stay faint: an opaque highlight can land on any lightness under
                    // an ambience. The stronger tint marks the own side; both are settable.
                    color: row.isOwn
                           ? Theme.rgba(appearance.ownBubbleColor.length > 0
                                        ? appearance.ownBubbleColor
                                        : Theme.highlightBackgroundColor,
                                        appearance.ownBubbleOpacity)
                           : Theme.rgba(appearance.otherBubbleColor.length > 0
                                        ? appearance.otherBubbleColor
                                        : Theme.highlightBackgroundColor,
                                        appearance.otherBubbleOpacity)
                    opacity: model.pending === true ? 0.5 : 1.0

                    Column {
                        id: bubbleColumn

                        // The widest a bubble's text may get before wrapping. Someone else's starts
                        // further right and gets correspondingly less.
                        property real maxTextWidth: timelineView.width * 0.82
                                                    - 2 * Theme.paddingMedium
                                                    - (row.isOwn ? 0 : page.avatarSize
                                                                      + Theme.paddingSmall)

                        /// How light it is behind a picture: the bubble's tint over the
                        /// ambience. The picture's own content is none of the frame's business.
                        readonly property real backgroundLightness: {
                            var fill = bubble.color
                            var tint = 0.299 * fill.r + 0.587 * fill.g + 0.114 * fill.b
                            var base = Theme.colorScheme === Theme.LightOnDark ? 0.0 : 1.0
                            return base * (1.0 - fill.a) + tint * fill.a
                        }

                        /// The opposite grey: dark on light, light on dark. A grey, not
                        /// a colour - a colour would be decoration, this is an edge.
                        readonly property color frameLine:
                            backgroundLightness > 0.5 ? Qt.rgba(0, 0, 0, 0.55)
                                                      : Qt.rgba(1, 1, 1, 0.55)

                        /// One device pixel: two would read as a border, one reads as
                        /// an edge.
                        readonly property real frameLineWidth:
                            Math.max(1, Math.round(Theme.pixelRatio))

                        /// Between the line and the picture, so the frame reads as a
                        /// frame and not as a rim the picture is pressed against.
                        readonly property real frameMat: Theme.paddingSmall / 2

                        anchors {
                            left: parent.left
                            top: parent.top
                            margins: Theme.paddingMedium
                        }
                        // No explicit width: a Column is as wide as its widest child, and every child
                        // derives its width from its own - which keeps the loops out.
                        spacing: Theme.paddingSmall / 2

                        // Which edge the children stand on: an own bubble grows leftwards, so its
                        // content hangs off the right edge. `x` from the parent's width, never width.
                        readonly property bool holdRight: row.isOwn

                        Label {
                            id: senderLabel

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            width: Math.min(implicitWidth, bubbleColumn.maxTextWidth)
                            visible: !row.isOwn
                            font.pixelSize: Theme.fontSizeExtraSmall
                            color: appearance.nameColor.length > 0
                                   ? appearance.nameColor : Theme.highlightColor
                            truncationMode: TruncationMode.Fade
                            textFormat: Text.PlainText
                            text: model.senderName || ""
                        }

                        // The quoted message, bar on the LEFT so name and text hang off it.
                        Item {
                            id: replyBlock

                            // On the column's child, not inside: there it was a no-op
                            // and the quote stayed left while an own bubble grew.
                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                            // Not for a thread row: the SDK marks every threaded event a reply to the
                            // previous one, which drew a quote no other client shows, one fetch each.
                            visible: !!model.replyTo
                                     && (model.threadRoot || "").length === 0
                            // Anchors, no `Row`: the row sized itself from these children
                            // while one of them took its share from the row.
                            width: bubbleColumn.maxTextWidth
                            height: Math.max(quoteTexts.height, quoteThumbFrame.height)


                            Rectangle {
                                id: quoteBar

                                anchors.left: parent.left
                                width: Theme.paddingSmall / 2
                                height: parent.height
                                radius: width / 2
                                color: Theme.rgba(Theme.highlightColor, 0.6)
                            }

                            // A quoted picture as a picture - its body is only the file name. In a frame
                            // of its own, which is also what a picture that has not arrived stands in.
                            Rectangle {
                                id: quoteThumbFrame

                                readonly property var quotedMedia: model.replyTo
                                                                   ? model.replyTo.media : null
                                readonly property string mediaKey: model.replyTo
                                                                   ? model.replyTo.eventId + "/quote" : ""

                                visible: !!quotedMedia
                                         && (model.replyTo.msgtype === "m.image"
                                             || model.replyTo.msgtype === "m.video")

                                anchors {
                                    left: quoteBar.right
                                    leftMargin: Theme.paddingSmall
                                    verticalCenter: parent.verticalCenter
                                }


                                /// The longest side of the picture. Square while the event names
                                /// no size - the only shape that claims nothing.
                                readonly property real box: Theme.itemSizeMedium
                                readonly property real aspect:
                                    quotedMedia && quotedMedia.width > 0 && quotedMedia.height > 0
                                    ? quotedMedia.width / quotedMedia.height : 1

                                // The event's own measurements, never the loaded picture's:
                                // `PreserveAspectFit` recomputes the implicit size from the given one.
                                width: visible ? (aspect >= 1 ? box : Math.round(box * aspect)) : 0
                                height: visible ? (aspect >= 1 ? Math.round(box / aspect) : box) : 0
                                radius: Theme.paddingSmall / 2
                                // The same edge as a picture in a message: the grey
                                // opposite the background it stands on.
                                color: Theme.rgba(Theme.primaryColor, 0.1)
                                border.width: bubbleColumn.frameLineWidth
                                border.color: bubbleColumn.frameLine

                                Image {
                                    id: quoteThumb

                                    readonly property var quotedMedia: quoteThumbFrame.quotedMedia
                                    readonly property string mediaKey: quoteThumbFrame.mediaKey

                                    // Inside the box, mat all round. The proportions are the
                                    // fill mode's business, not the layout's.
                                    anchors {
                                        fill: parent
                                        margins: quoteThumbFrame.border.width + bubbleColumn.frameMat
                                    }

                                    // The whole picture, not its middle: `PreserveAspectCrop` cuts away exactly
                                    // what a quote is meant to identify.
                                    fillMode: Image.PreserveAspectFit
                                    asynchronous: true
                                    // A fixed decode ceiling - a picture from outside must not ask for arbitrary
                                    // memory. Both dimensions means "scale to fit inside this".
                                    sourceSize.width: Theme.itemSizeMedium * 2
                                    sourceSize.height: Theme.itemSizeMedium * 2

                                    // Also on a change of key: the delegate is recycled while scrolling and would
                                    // keep the picture of the row it was last used for.
                                    Component.onCompleted: load()
                                    onMediaKeyChanged: {
                                        source = ""
                                        load()
                                    }

                                    function load() {
                                        // The frame carries the condition; this
                                        // one is inside it and always visible.
                                        if (!quoteThumbFrame.visible
                                                || mediaKey.length === 0) {
                                            return
                                        }
                                        var known = matrix.mediaPath(mediaKey)
                                        if (known.length > 0) {
                                            source = "file://" + known
                                        } else if (quotedMedia.thumbnailSource) {
                                            matrix.requestMedia(mediaKey, quotedMedia.thumbnailSource, false,
                                                                quotedMedia.size || 0)
                                        } else {
                                            matrix.requestMedia(mediaKey, quotedMedia.source, true,
                                                                quotedMedia.size || 0)
                                        }
                                    }

                                    Connections {
                                        target: matrix
                                        onMediaReady: {
                                            if (key === quoteThumb.mediaKey) {
                                                quoteThumb.source = "file://" + path
                                            }
                                        }
                                    }
                                }
                            }

                            Column {
                                id: quoteTexts

                                // Beside the picture, not above its top edge.
                                anchors {
                                    left: quoteThumbFrame.visible
                                          ? quoteThumbFrame.right : quoteBar.right
                                    leftMargin: Theme.paddingSmall
                                    verticalCenter: parent.verticalCenter
                                }

                                // What the labels may use once the bar, the
                                // picture and the spacing took their share.
                                readonly property real maxWidth: replyBlock.width
                                                                 - quoteBar.width
                                                                 - Theme.paddingSmall
                                                                 - (quoteThumbFrame.visible
                                                                    ? quoteThumbFrame.width + Theme.paddingSmall
                                                                    : 0)

                                Label {
                                    id: quoteSender

                                    width: Math.min(implicitWidth, quoteTexts.maxWidth)
                                    // An empty line above the quote reads as a glitch; the box collapses to the
                                    // body line until the sender is known.
                                    visible: text.length > 0
                                    font.pixelSize: Theme.fontSizeExtraSmall
                                    color: appearance.nameColor.length > 0
                                           ? appearance.nameColor : Theme.highlightColor
                                    truncationMode: TruncationMode.Fade
                                    textFormat: Text.PlainText
                                    // While the quoted event is being fetched its sender is unknown - the same
                                    // ellipsis the body uses says so.
                                    text: {
                                        if (!model.replyTo) {
                                            return ""
                                        }
                                        if ((model.replyTo.sender || "").length > 0) {
                                            return model.replyTo.sender
                                        }
                                        return model.replyTo.state === "loading" ? "…" : ""
                                    }
                                }

                                Label {
                                    id: quoteBody

                                    // Fixed to the full text width, never the own implicit width: with wrap and
                                    // elide that follows the set width in this Qt, and reading it back loops.
                                    width: quoteTexts.maxWidth
                                    font.pixelSize: Theme.fontSizeExtraSmall
                                    color: model.replyTo && model.replyTo.state === "error"
                                           ? Theme.errorColor : Theme.secondaryColor
                                    maximumLineCount: 2
                                    wrapMode: Text.Wrap
                                    elide: Text.ElideRight
                                    textFormat: Text.PlainText
                                    // "…" while fetching or for a quote with
                                    // no text; the error state names itself.
                                    text: {
                                        if (!model.replyTo) {
                                            return ""
                                        }
                                        if (model.replyTo.state === "error") {
                                            return qsTr("The quoted message cannot be loaded: it no longer exists or you are not allowed to see it.")
                                        }
                                        var body = model.replyTo.body || ""
                                        // Next to the picture the file name
                                        // is noise; a caption is not.
                                        if (quoteThumbFrame.visible && model.replyTo.media
                                                && body === model.replyTo.media.filename) {
                                            return ""
                                        }
                                        return body.length > 0 ? body : "…"
                                    }
                                    visible: text.length > 0
                                }
                            }

                            MouseArea {
                                anchors.fill: parent
                                // A quote whose original was never fetched has nowhere to
                                // go; the row says so and a tap would only look broken.
                                enabled: !!model.replyTo && model.replyTo.state !== "error"
                                onClicked: page.jumpToEvent(model.replyTo.eventId)
                                // The long press still belongs to the message, not to the
                                // quote — otherwise this corner of the bubble has no menu.
                                onPressAndHold: row.openMenu()
                            }
                        }

                        // A frame, so the picture does not run into the background.
                        // Sized to the reserved box, so it does not jump when the file lands.
                        Rectangle {
                            id: attachmentFrame

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            visible: row.hasPreview
                            width: visible ? attachment.width + 2 * (border.width + bubbleColumn.frameMat) : 0
                            height: visible ? attachment.height + 2 * (border.width + bubbleColumn.frameMat) : 0
                            radius: Theme.paddingSmall / 2
                            // The body a picture that has not arrived stands in; the
                            // picture covers it once it lands.
                            color: Theme.rgba(Theme.primaryColor, 0.1)
                            border.width: bubbleColumn.frameLineWidth
                            border.color: bubbleColumn.frameLine

                            // Attachments are fetched on demand and cached on disk, so
                            // scrolling past the same image does not download it twice.
                            Image {
                                id: attachment

                                x: attachmentFrame.border.width + bubbleColumn.frameMat
                                y: attachmentFrame.border.width + bubbleColumn.frameMat

                                visible: row.hasPreview
                                width: row.hasPreview
                                       ? bubbleColumn.maxTextWidth
                                         - 2 * (bubbleColumn.frameLineWidth + bubbleColumn.frameMat)
                                       : 0
                                height: visible
                                        ? (model.media && model.media.width > 0 && model.media.height > 0
                                           ? width * model.media.height / model.media.width
                                           : width * 0.75)
                                        : 0
                                fillMode: Image.PreserveAspectFit
                                asynchronous: true
                                // A ceiling on what is decoded, on both axes: a 64x200000 picture is a few
                                // kilobytes on the wire and gigabytes in memory. The bound is an area.
                                sourceSize.width: Math.round(bubbleColumn.maxTextWidth)
                                sourceSize.height: Math.round(bubbleColumn.maxTextWidth * 3)
                                source: {
                                    if (!row.hasPreview) {
                                        return ""
                                    }
                                    var known = matrix.mediaPath(model.id)
                                    return known.length > 0 ? "file://" + known : ""
                                }

                                Component.onCompleted: {
                                    if (!row.hasPreview || source != "") {
                                        return
                                    }
                                    // Only when asked, where the user said so: scrolling past a picture
                                    // is a request to the sender's server. The line stays tappable.
                                    if (!settings.autoLoadMedia) {
                                        return
                                    }
                                    // A sender's thumbnail is already small and, in encrypted rooms, the only one
                                    // there is. A video has no other preview at all.
                                    if (model.media.thumbnailSource) {
                                        matrix.requestMedia(model.id, model.media.thumbnailSource, false,
                                                            model.media.size || 0)
                                    } else {
                                        matrix.requestMedia(model.id, model.media.source, true,
                                                            model.media.size || 0)
                                    }
                                }

                                // Play affordance over the still.
                                Image {
                                    anchors.centerIn: parent
                                    visible: row.isVideo && attachment.source != ""
                                    source: "image://theme/icon-l-play?" + Theme.lightPrimaryColor
                                }

                                Connections {
                                    target: matrix
                                    onMediaReady: {
                                        if (key === model.id) {
                                            attachment.source = "file://" + path
                                        }
                                    }
                                }

                                BusyIndicator {
                                    anchors.centerIn: parent
                                    size: BusyIndicatorSize.Medium
                                    // Not while waiting for something that is not coming: a refused or failed
                                    // download left this turning for the life of the page.
                                    running: attachment.visible && attachment.source == ""
                                             && !page.failedMedia[model.id]
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    enabled: row.hasPreview && attachment.source != ""
                                    onClicked: row.isVideo
                                               ? page.openVideo(model.id, model.media)
                                               : page.openImage(model.id, model.media)
                                    // The delegate's own menu still has to be reachable.
                                    onPressAndHold: row.openMenu()
                                }

                            }
                        }

                        // Voice messages are played in place: switching to a page
                        // for a five second clip is more disruptive than useful.
                        Row {
                            id: audioRow

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            visible: row.isAudio
                            spacing: Theme.paddingMedium

                            IconButton {
                                anchors.verticalCenter: parent.verticalCenter
                                icon.source: page.playingId === model.id && audioPlayer.playbackState === Audio.PlayingState
                                             ? "image://theme/icon-m-pause"
                                             : "image://theme/icon-m-play"
                                onClicked: page.toggleAudio(model.id, model.media)
                            }

                            Label {
                                anchors.verticalCenter: parent.verticalCenter
                                width: Math.min(implicitWidth,
                                                bubbleColumn.maxTextWidth - Theme.itemSizeSmall)
                                font.pixelSize: Theme.fontSizeExtraSmall
                                color: Theme.secondaryColor
                                truncationMode: TruncationMode.Fade
                                text: page.playingId === model.id && audioPlayer.duration > 0
                                      ? Format.formatDuration(Math.round(audioPlayer.position / 1000),
                                                              Formatter.DurationShort)
                                        + " / "
                                        + Format.formatDuration(Math.round(audioPlayer.duration / 1000),
                                                                Formatter.DurationShort)
                                      : (model.body || qsTr("Voice message"))
                            }
                        }

                        Label {
                            id: bodyLabel

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            // Measured by the invisible twin, never by the own implicit width: with wrap
                            // that follows the laid-out width in this Qt, and reading it back loops.
                            width: Math.min(bodyMeasure.implicitWidth,
                                            bubbleColumn.maxTextWidth)
                            // A picture or a voice message shows itself; its
                            // caption is the one thing that still needs a line.
                            visible: (!row.hasPreview && !row.isAudio)
                                     || row.hasCaption

                            Label {
                                id: bodyMeasure

                                visible: false
                                // The twin measures what the label renders, so
                                // it has to parse the same markup.
                                textFormat: bodyLabel.textFormat
                                font.pixelSize: bodyLabel.font.pixelSize
                                font.italic: bodyLabel.font.italic
                                text: bodyLabel.text
                            }
                            // Plain text on purpose: a body is untrusted and AutoText renders anything
                            // HTML-shaped, `<img>` included. StyledText only after linkifyBody escaped it.
                            textFormat: row.richBody.length > 0
                                        ? Text.StyledText : Text.PlainText
                            linkColor: Theme.highlightColor
                            onLinkActivated: page.followLink(link)
                            wrapMode: Text.Wrap
                            font.pixelSize: Theme.fontSizeSmall
                            font.italic: model.kind !== "message"
                            color: {
                                if (model.kind !== "message") {
                                    return Theme.secondaryColor
                                }
                                var own = row.isOwn ? appearance.ownTextColor
                                                    : appearance.otherTextColor
                                return own.length > 0 ? own : Theme.primaryColor
                            }
                            MouseArea {
                                anchors.fill: parent
                                // Whatever carries a picture or film and got no preview - a video without a
                                // thumbnail, a picture whose size nobody declared. Otherwise it is text only.
                                enabled: (row.isVideo || row.isImage) && !row.hasPreview
                                         && row.saneDimensions
                                onClicked: {
                                    if (row.isVideo) {
                                        page.openVideo(model.id, model.media)
                                        return
                                    }
                                    page.openImage(model.id, model.media)
                                }
                                onPressAndHold: row.openMenu()
                            }

                            text: {
                                if (row.hasPreview || row.isAudio) {
                                    return row.captionText
                                }
                                if (model.kind === "undecryptable") {
                                    return page.undecryptableText(model.utdCause)
                                }
                                if (model.kind === "redacted") {
                                    return qsTr("Message deleted")
                                }
                                if (row.isImage) {
                                    // No preview, so the row says what it is
                                    // and that it can be opened.
                                    return "🖼 " + (model.body || qsTr("Picture"))
                                }
                                if (row.isFile) {
                                    return "📎 " + (model.body || "")
                                }
                                if (row.richBody.length > 0) {
                                    return row.richBody
                                }
                                return model.body || ""
                            }
                        }

                        // What the SDK will not vouch for: red is a message that is not what it
                        // claims to be, grey one it cannot check. Both silent otherwise.
                        Label {
                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                            width: Math.min(implicitWidth, bubbleColumn.maxTextWidth)
                            visible: row.isBubble && !!model.shield
                                     && page.shieldEventId === model.eventId
                            font.pixelSize: Theme.fontSizeExtraSmall
                            color: model.shield && model.shield.level === "red"
                                   ? page.shieldRedColor : page.shieldGreyColor
                            textFormat: Text.PlainText
                            text: page.shieldText(model.shield)
                        }

                        // The way from one sentence to all of them, shown with the sentence: a mark
                        // that is never tapped needs no glossary either.
                        Label {
                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                            visible: row.isBubble && !!model.shield
                                     && page.shieldEventId === model.eventId
                            width: Math.min(implicitWidth, bubbleColumn.maxTextWidth)
                            font.pixelSize: Theme.fontSizeExtraSmall
                            color: Theme.highlightColor
                            textFormat: Text.PlainText
                            text: qsTr("What do these marks mean?")

                            MouseArea {
                                anchors.fill: parent
                                anchors.margins: -Theme.paddingSmall
                                onClicked: pageStack.push(
                                               Qt.resolvedUrl("ShieldGlossaryPage.qml"))
                            }
                        }

                        // Thread marker: the root shows the reply count, a
                        // reply shown inline names its thread. Both open it.
                        Label {
                            id: threadLabel

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            visible: row.isBubble
                                     && (model.threadCount > 0
                                         || (model.threadRoot || "").length > 0)
                            width: Math.min(implicitWidth, bubbleColumn.maxTextWidth)
                            font.pixelSize: Theme.fontSizeExtraSmall
                            color: threadArea.pressed ? Theme.secondaryHighlightColor
                                                      : Theme.highlightColor
                            text: model.threadCount > 0
                                  ? qsTr("Thread · %1").arg(model.threadCount)
                                  : qsTr("In thread")

                            MouseArea {
                                id: threadArea
                                anchors {
                                    fill: parent
                                    margins: -Theme.paddingSmall
                                }
                                onClicked: pageStack.push(
                                               Qt.resolvedUrl("ThreadPage.qml"),
                                               {
                                                   roomId: page.roomId,
                                                   roomName: page.roomName,
                                                   rootEventId: model.threadCount > 0
                                                                ? model.eventId
                                                                : model.threadRoot,
                                                   encrypted: page.encrypted
                                               })
                                onPressAndHold: row.openMenu()
                            }
                        }

                        // Reactions, and only where there are any. A Row, not a Flow: a Flow's width
                        // comes from its own implicit width, which is the loop that drew chips on text.
                        Loader {
                            id: reactionBar

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            active: row.hasReactions
                            visible: active
                            // No size here: a Loader that has one resizes what it loaded, so taking the
                            // size from that item is a circle - and Qt answers a circle with zero.
                            sourceComponent: Row {
                                // No padding properties: they arrived in QtQuick 2.7 and 5.6 is the ceiling.
                                // The column's spacing separates the bar from the text.
                                spacing: Theme.paddingSmall

                                Repeater {
                                    model: row.reactionsShown

                                    BackgroundItem {
                                        width: chip.width + 2 * Theme.paddingSmall
                                        height: chip.height + Theme.paddingSmall
                                        // The delegate's model is out of scope inside the Loader; the row hands the
                                        // event id over instead.
                                        onClicked: {
                                            // Drawn is filtered and cut to 32 characters, sent is the raw key: a stranger
                                            // can hide text behind zero-width characters and have it signed by one tap.
                                            var sendKey = modelData.sendKey || modelData.key
                                            if (!modelData.mine && sendKey !== modelData.key) {
                                                page.showNotice(qsTr("This reaction hides text and was not sent"))
                                                return
                                            }
                                            matrix.toggleReaction(row.rowEventId, sendKey)
                                        }
                                        // Held down, the chip says who set it. It takes the long press for the width
                                        // of a chip; the rest of the bubble keeps the message's menu.
                                        onPressAndHold: page.showReactors(
                                                            row.rowEventId,
                                                            modelData.sendKey || modelData.key,
                                                            modelData.key)

                                        Rectangle {
                                            anchors.fill: parent
                                            radius: Theme.paddingSmall
                                            // Ours is the one worth telling
                                            // apart; the rest carry the bubble.
                                            color: modelData.mine
                                                   ? Theme.rgba(Theme.highlightColor, 0.35)
                                                   : Theme.rgba(Theme.secondaryColor, 0.15)
                                        }

                                        Row {
                                            id: chip

                                            anchors.centerIn: parent
                                            spacing: Theme.paddingSmall / 2

                                            // One rule for drawing an emoji, in `EmojiItem`: the user's picture where
                                            // there is one, otherwise the character.
                                            EmojiItem {
                                                anchors.verticalCenter: parent.verticalCenter
                                                character: modelData.key
                                                // Half again the icon size it was drawn at: on a small screen an emoji that
                                                // size is a coloured speck.
                                                size: Math.round(Theme.iconSizeExtraSmall * 1.5)
                                            }

                                            Label {
                                                anchors.verticalCenter: parent.verticalCenter
                                                visible: modelData.count > 1
                                                font.pixelSize: Theme.fontSizeExtraSmall
                                                text: modelData.count
                                            }
                                        }
                                    }
                                }

                                // A room can carry more distinct reactions than a bubble has room for; the
                                // rest are counted rather than pushing the bubble off screen.
                                Label {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: row.reactionsHidden > 0
                                    font.pixelSize: Theme.fontSizeExtraSmall
                                    color: Theme.secondaryColor
                                    text: "+" + row.reactionsHidden
                                }
                            }
                        }

                        // The time, and how many have read this far. An Item because the read mark is
                        // a drawn icon and an icon cannot sit in a string.
                        Item {
                            id: metaLine

                            // Right-aligned inside whatever the bubble ended up
                            // being, so it reads from the widest sibling.
                            width: Math.max(bodyLabel.visible ? bodyLabel.width : 0,
                                            attachment.width,
                                            audioRow.visible ? audioRow.width : 0,
                                            senderLabel.visible ? senderLabel.width : 0,
                                            replyBlock.visible ? replyBlock.width : 0,
                                            threadLabel.visible ? threadLabel.width : 0,
                                            // The names below can be the widest thing in the bubble; without them here
                                            // the time would stand right of nothing.
                                            readersLabel.visible ? readersLabel.width : 0,
                                            reactionBar.visible ? reactionBar.width : 0,
                                            metaContent.width)
                            height: metaContent.height

                            Row {
                                id: metaContent

                                anchors.right: parent.right
                                spacing: Theme.paddingSmall

                                // The shield as a mark, not a sentence: the report was about the repetition.
                                // Shape carries the level - triangle warns, dot is unchecked.
                                Item {
                                    id: shieldMark

                                    visible: row.isBubble && !!model.shield
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: visible ? shieldGlyph.width : 0
                                    height: shieldGlyph.height

                                    Label {
                                        id: shieldGlyph

                                        text: model.shield
                                              && model.shield.level === "red" ? "\u25B2" : "\u25CF"
                                        // A step larger than the time beside it, like the read-by eye over its digit:
                                        // a mark at text size is read as text.
                                        font.pixelSize: Theme.fontSizeExtraSmall
                                        color: model.shield
                                               && model.shield.level === "red"
                                               ? page.shieldRedColor
                                               : page.shieldGreyColor
                                    }

                                    MouseArea {
                                        // Larger than the glyph: a triangle at
                                        // tiny size is not a touch target.
                                        anchors.centerIn: parent
                                        width: Theme.itemSizeExtraSmall
                                        height: Theme.itemSizeExtraSmall
                                        onClicked: page.showShield(model.eventId)
                                    }
                                }

                                Label {
                                    id: metaLabel

                                    anchors.verticalCenter: parent.verticalCenter
                                    font.pixelSize: Theme.fontSizeTiny
                                    // A message that did not get out is the one thing in this line worth a colour.
                                    color: model.sendState === "failed"
                                           ? Theme.errorColor : Theme.secondaryColor
                                    // Only messages carry a timestamp; the shared delegate instantiates this
                                    // label for every row regardless.
                                    text: row.isBubble
                                          ? (matrix.pinnedEventIds.indexOf(model.eventId) >= 0 ? "📌 " : "")
                                            + (model.sendState === "failed"
                                               ? qsTr("not sent") + " · " : "")
                                            + (model.edited === true ? qsTr("edited") + " · " : "")
                                            + Format.formatDate(new Date(model.timestamp), Formatter.TimeValue)
                                          : ""
                                }

                                // On the newest own message the others have read up to - a receipt usually
                                // hangs on their own message. An eye and a number, not words: the line is full.
                                Item {
                                    id: readMark

                                    visible: model.readMark === true && model.readMarkBy > 0
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: readMarkRow.width
                                    height: readMarkRow.height

                                    Row {
                                        id: readMarkRow

                                        spacing: Theme.paddingSmall / 2

                                        EyeIcon {
                                            anchors.verticalCenter: parent.verticalCenter
                                            // A step larger than the digit next to it, the way the composer's icon stands
                                            // over its text.
                                            size: Math.round(Theme.fontSizeTiny * 1.7)
                                            color: Theme.secondaryColor
                                        }

                                        Label {
                                            anchors.verticalCenter: parent.verticalCenter
                                            font.pixelSize: Theme.fontSizeTiny
                                            color: Theme.secondaryColor
                                            text: model.readMarkBy
                                        }
                                    }

                                    // Eye, number and a good deal of air: the mark is a couple of millimetres and
                                    // a finger is not. Negative margins grow the target without moving anything.
                                    MouseArea {
                                        anchors {
                                            fill: parent
                                            margins: -Theme.paddingMedium
                                        }
                                        onClicked: page.showReaders(model.eventId)
                                        onPressAndHold: page.showReaders(model.eventId)
                                    }
                                }
                            }
                        }

                        // Who they are, as one line: a name per row would push the conversation off
                        // the screen for an aside.
                        Label {
                            id: readersLabel

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            visible: page.namesEventId === model.eventId
                                     && page.namesText.length > 0
                            // The names widen the bubble to the same limit as every other text, and only
                            // what is longer wraps. Measured by the twin - the own implicit width loops.
                            width: Math.min(readersMeasure.implicitWidth,
                                            bubbleColumn.maxTextWidth)
                            // Left, unlike the time above: this runs over several lines, and a ragged
                            // left edge is read line by line instead of at a glance.
                            horizontalAlignment: Text.AlignLeft
                            wrapMode: Text.Wrap
                            font.pixelSize: Theme.fontSizeTiny
                            color: Theme.secondaryColor
                            textFormat: Text.PlainText
                            // Only the row the names belong to carries them: the string is the page's, so
                            // otherwise every row would hold it and its twin would lay it out.
                            text: readersLabel.visible ? page.namesText : ""

                            Label {
                                id: readersMeasure

                                visible: false
                                textFormat: Text.PlainText
                                font.pixelSize: readersLabel.font.pixelSize
                                text: readersLabel.text
                            }
                        }
                    }
                }

                Rectangle {
                    visible: row.showMarker
                    anchors {
                        left: parent.left
                        right: parent.right
                        leftMargin: Theme.horizontalPageMargin
                        rightMargin: Theme.horizontalPageMargin
                        top: parent.top
                        topMargin: row.contentHeight
                                   - Math.round(Theme.paddingLarge / 2)
                    }
                    height: 1
                    color: Theme.rgba(Theme.highlightColor, 0.6)
                }

                Label {
                    id: dayLabel

                    visible: model.kind === "date"
                    anchors {
                        horizontalCenter: parent.horizontalCenter
                        top: parent.top
                        topMargin: Theme.paddingMedium
                    }
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryColor
                    text: model.kind === "date"
                          ? Format.formatDate(new Date(model.timestamp), Formatter.DateMedium)
                          : ""
                }

                Label {
                    id: systemLabel

                    visible: row.isSystem
                    anchors {
                        horizontalCenter: parent.horizontalCenter
                        top: parent.top
                        topMargin: Theme.paddingMedium
                    }
                    // A system line can be long; keep it inside the page.
                    width: Math.min(implicitWidth, timelineView.width - 2 * Theme.horizontalPageMargin)
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    font.italic: true
                    color: Theme.secondaryColor
                    horizontalAlignment: Text.AlignHCenter
                    text: row.isSystem ? row.systemText() : ""
                }
            }

            onCountChanged: {
                page.fillScreen()
                if (page.jumpTargetId.length > 0) {
                    page.tryJump()
                } else if (page.followTail) {
                    tailTimer.restart()
                    // Read where it is actually read: marking only on entering left a message
                    // that arrived while the room was open unread in the chat list.
                    readTimer.restart()
                }
            }

            onMovementEnded: {
                page.followTail = atYEnd
                // A hand on the list outranks the marker search: being carried off to a jump
                // one has just scrolled away from is worse than not being taken there.
                page.unreadFromId = ""
                unreadRetry.stop()
                // Scrolling down to the newest message is the other moment
                // something becomes read.
                if (atYEnd) {
                    readTimer.restart()
                }
            }

            // Reaching the top is the natural moment to fetch older messages, so the
            // fill's give-up does not apply - only a running pagination or a loaded start.
            onAtYBeginningChanged: {
                if (atYBeginning && count > 0 && !matrix.paginating
                        && !matrix.timelineAtStart) {
                    matrix.loadOlder()
                }
            }

            // Fill and tail both hang off a changing content height: a row that grows
            // after layout pushes the end out of view without changing the count.
            onContentHeightChanged: {
                // An open menu grows the content by its own height, which is not
                // new content: following the tail takes the pressed row off the top.
                if (page.openMenus > 0) {
                    return
                }
                page.fillScreen()
                // A jump still looking for its row outranks the tail: the rows of the very
                // pagination it asked for would drag the view back to the end.
                if (page.followTail && page.jumpTargetId.length === 0) {
                    tailTimer.restart()
                }
            }

            // The viewport loses height at its bottom edge while Qt keeps the content's
            // top, so the newest message slides away. Shrinking, push the content down.
            onHeightChanged: {
                var lost = page.lastTimelineHeight - height
                page.lastTimelineHeight = height
                if (lost > 0) {
                    contentY = Math.min(contentY + lost,
                                        originY + Math.max(0, contentHeight - height))
                } else if (lost < 0 && page.followTail
                           && page.jumpTargetId.length === 0) {
                    tailTimer.restart()
                }
            }

            // Empty, still loading and could not be opened are three states:
            // without the third the spinner ran for ever.
            ViewPlaceholder {
                enabled: timelineView.count === 0 && !page.invited && matrix.timelineReady
                text: page.openFailed ? qsTr("The conversation could not be loaded")
                                      : qsTr("No messages")
                hintText: page.openFailed ? page.openFailedReason : ""
            }

            BusyIndicator {
                anchors.centerIn: parent
                size: BusyIndicatorSize.Medium
                running: timelineView.count === 0 && !page.invited && !matrix.timelineReady
            }

            VerticalScrollDecorator { }
        }

        // Feedback for "copy room link". The pull-down has closed by the time
        // the core answers, so the confirmation belongs over the conversation.
        Label {
            id: linkHint

            anchors {
                top: timelineView.top
                topMargin: Theme.paddingLarge
                horizontalCenter: timelineView.horizontalCenter
            }
            visible: false
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.primaryColor
            textFormat: Text.PlainText
            text: qsTr("Room link copied")

            Rectangle {
                anchors.fill: parent
                anchors.margins: Theme.paddingSmall
                anchors.leftMargin: -Theme.paddingMedium
                anchors.rightMargin: -Theme.paddingMedium
                anchors.topMargin: -Theme.paddingSmall
                anchors.bottomMargin: -Theme.paddingSmall
                z: -1
                radius: Theme.paddingSmall
                color: Theme.rgba(Theme.highlightDimmerColor, 0.9)
            }
        }

        // One player for the page: two voice messages never play at once, and the
        // delegate is recycled while scrolling.
        Audio {
            id: audioPlayer

            onStopped: {
                if (status === Audio.EndOfMedia) {
                    page.playingId = ""
                }
            }
        }

        // Positioning right after a model change lands on the old geometry, so it
        // is deferred by one event loop turn.
        Timer {
            id: tailTimer

            interval: 1
            onTriggered: {
                // Never against a hand on the list: rows arriving from a pagination tell the
                // tail to follow, which jumped to the end mid-swipe.
                if (timelineView.moving || timelineView.dragging) {
                    return
                }
                timelineView.positionViewAtEnd()
            }
        }

        // Composer, or the invitation prompt when the room has not been joined.
        Item {
            id: composer

            anchors {
                left: parent.left
                right: parent.right
                bottom: parent.bottom
            }
            height: page.invited
                    ? joinButton.height + 2 * Theme.paddingLarge
                    : composerColumn.height + Theme.paddingSmall

            Button {
                id: joinButton

                visible: page.invited
                anchors.centerIn: parent
                text: qsTr("Accept invitation")
                onClicked: {
                    matrix.joinRoom(page.roomId)
                    page.invited = false
                    matrix.openRoom(page.roomId)
                }
            }

            Column {
                id: composerColumn

                visible: !page.invited
                anchors {
                    left: parent.left
                    right: parent.right
                    bottom: parent.bottom
                }

                // While recording, the elapsed time replaces the hints above the
                // field so it is obvious that the microphone is live.
                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    visible: matrix.recorder.recording
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.errorColor
                    text: qsTr("Recording… %1 s").arg(Math.round(matrix.recorder.duration / 1000))
                }

                Row {
                    width: parent.width
                    visible: page.replyingEventId.length > 0
                    spacing: Theme.paddingSmall

                    Label {
                        x: Theme.horizontalPageMargin
                        width: parent.width - Theme.horizontalPageMargin - cancelReply.width
                        anchors.verticalCenter: parent.verticalCenter
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.highlightColor
                        truncationMode: TruncationMode.Fade
                        text: qsTr("Reply to %1").arg(page.replyingTo)
                    }

                    IconButton {
                        id: cancelReply

                        icon.source: "image://theme/icon-m-clear"
                        onClicked: page.cancelReply()
                    }
                }

                Row {
                    width: parent.width
                    visible: page.editingEventId.length > 0
                    spacing: Theme.paddingSmall

                    Label {
                        x: Theme.horizontalPageMargin
                        width: parent.width - Theme.horizontalPageMargin - cancelEdit.width
                        anchors.verticalCenter: parent.verticalCenter
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.highlightColor
                        truncationMode: TruncationMode.Fade
                        text: qsTr("Editing message")
                    }

                    IconButton {
                        id: cancelEdit

                        icon.source: "image://theme/icon-m-clear"
                        onClicked: page.cancelEdit()
                    }
                }

                Item {
                    width: parent.width
                    height: Math.max(Theme.itemSizeMedium, messageField.height + Theme.paddingMedium)

                    IconButton {
                        id: attachButton

                        anchors {
                            left: parent.left
                            leftMargin: Theme.paddingMedium
                            verticalCenter: parent.verticalCenter
                        }
                        visible: page.editingEventId.length === 0
                        icon.source: "image://theme/icon-m-attach"
                        // The same step down as the face beside it. Both are offers; the send arrow
                        // is the action and brightens by itself once there is something to send.
                        icon.opacity: Theme.opacityLow
                        onClicked: pageStack.push(contentPicker)
                    }

                    TextArea {
                        id: messageField

                        anchors {
                            // Straight after the paper clip, and at the page margin where that is hidden.
                            // The lock that stood in this gap says its piece at the top of the room now.
                            left: attachButton.visible ? attachButton.right
                                                       : parent.left
                            leftMargin: attachButton.visible
                                        ? 0 : Theme.horizontalPageMargin
                            right: emojiButton.left
                            verticalCenter: parent.verticalCenter
                        }
                        // The field brings a page margin of its own, which in a row with a button at
                        // each end is margin twice over. The row's spacing does that job.
                        textMargin: 0
                        placeholderText: page.editingEventId.length > 0
                                         ? qsTr("New text")
                                         : qsTr("Message")
                        labelVisible: false

                        // Grow with the text, but stop before the composer eats the screen: past the
                        // cap the field scrolls internally instead of pushing lines behind the header.
                        height: Math.min(implicitHeight, Theme.itemSizeMedium * 3)
                    }

                    // Hold to record, release to send. A tap-to-start button
                    // invites accidental minute-long recordings.
                    IconButton {
                        id: recordButton

                        anchors {
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin
                            verticalCenter: parent.verticalCenter
                        }
                        visible: settings.voiceMessages
                                 && page.editingEventId.length === 0
                                 && messageField.text.trim().length === 0
                                 && !messageField.inputMethodComposing
                        icon.source: matrix.recorder.recording
                                     ? "image://theme/icon-m-mic?" + Theme.errorColor
                                     : "image://theme/icon-m-mic"

                        onPressAndHold: matrix.recorder.start()
                        onReleased: {
                            if (matrix.recorder.recording) {
                                matrix.recorder.stop()
                                page.followTail = true
                            }
                        }
                        onCanceled: matrix.recorder.cancel()
                    }

                    // Microphone and send share the right slot - never both - so the face does
                    // not move when one takes over. Not an IconButton: the icon is drawn.
                    MouseArea {
                        id: emojiButton

                        anchors {
                            right: sendButton.left
                            rightMargin: Theme.paddingSmall
                            verticalCenter: parent.verticalCenter
                        }
                        width: Theme.itemSizeSmall
                        height: Theme.itemSizeSmall

                        onClicked: page.pickEmoji()

                        FaceIcon {
                            anchors.centerIn: parent
                            // Measured against the theme's icons, not set to their nominal size: at
                            // iconSizeMedium the face came out a third taller than the arrow beside it.
                            size: Math.round(Theme.iconSizeMedium * 0.78)
                            color: emojiButton.pressed ? Theme.highlightColor
                                                       : Theme.primaryColor
                        }
                    }

                    IconButton {
                        id: sendButton

                        anchors {
                            right: parent.right
                            rightMargin: Theme.horizontalPageMargin
                            verticalCenter: parent.verticalCenter
                        }
                        visible: !recordButton.visible
                        // `inputMethodComposing` is the uncommitted word and what Qt 5.6 offers -
                        // `preeditText` arrives in 5.7.
                        enabled: messageField.text.trim().length > 0
                                 || messageField.inputMethodComposing
                        icon.source: "image://theme/icon-m-send"
                        // The arrow fills barely half its box and reads as a smaller button. Scaled
                        // rather than replaced: the artwork is the platform's.
                        icon.scale: 1.25
                        onClicked: page.submit()
                    }
                }
            }
        }
    }


    // One picker for everything: pictures, documents, downloads. Two separate
    // entry points would only make the user guess which one holds their file.
    Component {
        id: contentPicker

        ContentPickerPage {
            allowedOrientations: Orientation.All
            // Only remembered here, never navigated from here: the picker is running its
            // own transition and answered a `replace` with "cannot pop". The timer opens it.
            onSelectedContentPropertiesChanged: {
                page.pendingAttachment = {
                    path: selectedContentProperties.filePath,
                    mimeType: selectedContentProperties.mimeType
                }
                attachmentWait.restart()
            }
        }
    }

    // The file picked, until the conversation is back on top and the send page
    // can be opened over it.
    property var pendingAttachment: null

    Timer {
        id: attachmentWait

        property int tries: 0

        interval: 60
        repeat: true
        onTriggered: {
            if (!page.pendingAttachment) {
                stop()
                tries = 0
                return
            }
            if (page.status === PageStatus.Active) {
                stop()
                tries = 0
                var picked = page.pendingAttachment
                page.pendingAttachment = null
                // What was typed goes along as the caption - one message. The field is
                // cleared so it cannot be sent twice, and refilled if the send is called off.
                var typed = messageField.text
                messageField.text = ""
                pageStack.push(Qt.resolvedUrl("SendMediaPage.qml"), {
                    path: picked.path,
                    mimeType: picked.mimeType,
                    caption: typed,
                    replyTo: page.replyingEventId,
                    replySender: page.replyingTo,
                    replyBody: page.replyingBody,
                    // Handed back here rather than sent from the dialog: the unverified-recipient
                    // warning lives on this page, and an attachment went past it.
                    send: function (path, mimeType, caption, replyTo) {
                        page.pendingAction = {
                            "kind": "sendMedia",
                            "path": path,
                            "mimeType": mimeType,
                            "caption": caption,
                            "replyTo": replyTo
                        }
                    },
                    // Only reached where this page does not take the send over itself; kept so
                    // the dialog still works for any other caller.
                    afterSend: function () {
                        page.clearReplyState()
                        page.followTail = true
                    },
                    afterCancel: function (text) {
                        messageField.text = text
                    }
                })
                return
            }
            // The picker did not come back by itself; ask for the way back
            // rather than waiting for it forever.
            if (++tries > 25) {
                tries = 0
                // Stopped, not just reset: otherwise "give up" meant asking the stack to pop
                // every second and a half for the life of the page.
                stop()
                page.pendingAttachment = null
                pageStack.pop(page)
            }
        }
    }

    // Recipients with unverified devices the user has not chosen to trust yet.
    // Empty means nothing stands in the way of sending.
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
        // The word being typed is in the input method's preedit, not in `text`: whoever
        // reaches straight for send sent everything but the last word.
        Qt.inputMethod.commit()
        // In an encrypted room, warn once before a message reaches a recipient with
        // unverified devices. The dialog handles "send anyway" and calls back.
        var pending = pendingUnverified()
        if (pending.length > 0) {
            var dialog = pageStack.push(Qt.resolvedUrl("UnverifiedRecipientsDialog.qml"),
                                        { users: pending })
            dialog.accepted.connect(page.doSubmit)
            return
        }
        doSubmit()
    }

    // The keyboard after a message has gone out: what one wants to see is the
    // conversation, not the emptied field. The setting keeps it up.
    function afterSend() {
        page.followTail = true
        if (settings.hideKeyboardOnSend) {
            messageField.focus = false
        }
    }

    function doSubmit() {
        if (page.editingEventId.length > 0) {
            matrix.editMessage(page.editingEventId, messageField.text)
            // cancelEdit drops the focus by itself - an edit that is done is
            // done, whatever the setting says.
            page.cancelEdit()
            return
        }
        if (page.replyingEventId.length > 0) {
            matrix.replyToMessage(page.replyingEventId, messageField.text)
            page.cancelReply()
            page.afterSend()
            return
        }
        matrix.sendMessage(messageField.text)
        messageField.text = ""
        page.afterSend()
    }

    /// An attachment through the same gate a typed message goes through. What
    /// follows a send happens after it, so calling off at the warning changes nothing.
    function submitMedia(action) {
        // Not while the stack is moving: Silica drops a push issued during a
        // transition, and the picture with its caption was simply gone.
        if (pageStack.busy) {
            page.deferredMedia = action
            return
        }
        var pending = pendingUnverified()
        if (pending.length > 0) {
            var dialog = pageStack.push(Qt.resolvedUrl("UnverifiedRecipientsDialog.qml"),
                                        { users: pending })
            // A push that was refused after all hands the attachment back
            // rather than dropping it.
            if (!dialog) {
                page.deferredMedia = action
                return
            }
            dialog.accepted.connect(function() { page.sendMediaNow(action) })
            dialog.rejected.connect(function() { page.mediaDeclined() })
            return
        }
        page.sendMediaNow(action)
    }

    /// An attachment waiting for the page stack to stand still. Never dropped:
    /// it is the only copy of what the user chose.
    property var deferredMedia: null

    /// The user said no to the warning. Nothing is sent, and the attachment is
    /// let go of - it was their decision, not a lost message.
    function mediaDeclined() {
        page.deferredMedia = null
    }

    Connections {
        target: pageStack
        onBusyChanged: {
            if (!pageStack.busy && page.deferredMedia
                    && page.status === PageStatus.Active) {
                var action = page.deferredMedia
                page.deferredMedia = null
                page.submitMedia(action)
            }
        }
    }

    function sendMediaNow(action) {
        matrix.sendMedia(action.path, action.mimeType, action.caption, action.replyTo)
        page.clearReplyState()
        page.followTail = true
    }

    function beginReply(eventId, sender, body) {
        page.replyingEventId = eventId
        page.replyingTo = sender && sender.length > 0 ? sender : body
        // Kept separately from replyingTo: the send page shows sender and text
        // as two lines, the way the quote in the conversation does.
        page.replyingBody = body || ""
        page.editingEventId = ""
        messageField.forceActiveFocus()
    }

    function cancelReply() {
        page.replyingEventId = ""
        page.replyingTo = ""
        page.replyingBody = ""
        messageField.text = ""
    }

    // After an attachment went out as a reply. Unlike `cancelReply` this leaves
    // the composer alone - a half-typed message is not part of what was sent.
    function clearReplyState() {
        page.replyingEventId = ""
        page.replyingTo = ""
        page.replyingBody = ""
    }

    function toggleAudio(itemId, media) {
        if (page.playingId === itemId) {
            if (audioPlayer.playbackState === Audio.PlayingState) {
                audioPlayer.pause()
            } else {
                audioPlayer.play()
            }
            return
        }

        var key = itemId + "/full"
        var known = matrix.mediaPath(key)
        page.playingId = itemId
        if (known.length > 0) {
            audioPlayer.source = "file://" + known
            audioPlayer.play()
            return
        }
        page.pendingPlayKey = key
        matrix.requestMedia(key, media.source, false, media.size || 0)
    }

    function openVideo(itemId, media) {
        var key = itemId + "/full"
        var known = matrix.mediaPath(key)
        matrix.requestMedia(key, media.source, false, media.size || 0)
        pageStack.push(Qt.resolvedUrl("VideoPage.qml"), {
                           mediaKey: key,
                           source: known.length > 0 ? "file://" + known : "",
                           fileName: media.filename || ""
                       })
    }

    // Forwarding re-sends rather than references: the target room encrypts under
    // its own keys, so the file is fetched first and the page opens when it lands.
    function forwardAttachment(item) {
        if (!item || !item.media) {
            return
        }
        var key = item.id + "/full"
        var mime = item.media.mimetype || ""
        var known = matrix.mediaPath(key)
        if (known.length > 0) {
            pageStack.push(Qt.resolvedUrl("ForwardPage.qml"), {
                               path: "file://" + known,
                               mimeType: mime
                           })
            return
        }
        page.pendingForwardKey = key
        page.pendingForwardMime = mime
        matrix.requestMedia(key, item.media.source, false, item.media.size || 0)
    }

    function saveAttachment(item) {
        var key = item.id + "/full"
        var known = matrix.mediaPath(key)
        if (known.length > 0) {
            page.storeFile(known, item)
            return
        }
        page.pendingSaveKey = key
        page.pendingSaveIsImage = item.msgtype === "m.image"
        page.pendingSaveName = item.media ? (item.media.filename || "") : ""
        matrix.requestMedia(key, item.media.source, false, item.media.size || 0)
    }

    // Opens a message's thread, starting one where there is none. The landscape
    // action page calls this too.
    function openThread(eventId) {
        pageStack.push(Qt.resolvedUrl("ThreadPage.qml"), {
                           roomId: page.roomId,
                           roomName: page.roomName,
                           rootEventId: eventId,
                           encrypted: page.encrypted
                       })
    }

    // Choosing a reaction for a message. The dialog hands the character back;
    // sending it is a toggle, so choosing the one already given takes it back.
    function pickReaction(eventId) {
        var dialog = pageStack.push(Qt.resolvedUrl("ReactionDialog.qml"),
                                    { eventId: eventId })
        dialog.accepted.connect(function() {
            if (dialog.key.length > 0) {
                matrix.toggleReaction(eventId, dialog.key)
            }
        })
    }

    // Names under a message, unfolded on demand: readers behind the eye, or who
    // set one reaction. One line and one state for both - the rows are the cost.
    property string namesEventId: ""

    // Which message shows its authenticity sentence. The mark stands on every
    // affected message; the sentence is one at a time, on request.
    property string shieldEventId: ""

    // The two levels in the security page's colours. Grey at the size of a mark
    // is indistinguishable from the text, so the orange is the project's own.
    readonly property color shieldRedColor: Theme.errorColor
    readonly property color shieldGreyColor:
        SecurityStatus.color(SecurityStatus.ORANGE, Theme,
                             Theme.colorScheme === Theme.LightOnDark)
    /// Which reaction the names belong to; empty when they are the readers.
    property string namesKey: ""
    /// What stands in front of them - the reaction itself, so the line says
    /// what it is a list of without a word.
    property string namesPrefix: ""
    property string namesText: ""

    function clearNames() {
        page.namesEventId = ""
        page.namesKey = ""
        page.namesPrefix = ""
        page.namesText = ""
    }

    /// Toggle: a second tap on the same mark puts the sentence away again.
    function showShield(eventId) {
        page.shieldEventId = page.shieldEventId === eventId ? "" : eventId
    }

    function showReaders(eventId) {
        if (page.namesEventId === eventId && page.namesKey.length === 0) {
            page.clearNames()
            return
        }
        page.clearNames()
        page.namesEventId = eventId
        matrix.loadReaders(eventId)
    }

    /// Press and hold on a reaction: who set it. The key that goes to the core is
    /// the one the row sends with, not the one it draws.
    function showReactors(eventId, sendKey, shownKey) {
        if (page.namesEventId === eventId && page.namesKey === sendKey) {
            page.clearNames()
            return
        }
        page.clearNames()
        page.namesEventId = eventId
        page.namesKey = sendKey
        page.namesPrefix = shownKey + " "
        matrix.loadReactors(eventId, sendKey)
    }

    Connections {
        target: matrix
        onRoomLinkReady: {
            if (link.length === 0) {
                return
            }
            Clipboard.text = link
            linkHint.visible = true
            linkHintTimer.restart()
        }
    }

    Timer {
        id: linkHintTimer

        interval: 2500
        onTriggered: linkHint.visible = false
    }

    Connections {
        target: matrix
        onReadersReady: {
            if (eventId !== page.namesEventId || page.namesKey.length > 0) {
                return
            }
            var names = []
            for (var i = 0; i < readers.length; i++) {
                names.push(readers[i].name)
            }
            page.namesText = names.join(", ")
        }

        onReactorsReady: {
            if (eventId !== page.namesEventId || key !== page.namesKey) {
                return
            }
            var who = []
            for (var i = 0; i < reactors.length; i++) {
                who.push(reactors[i].name)
            }
            // Nobody to name: fold the line away rather than leave the reaction standing
            // alone, which reads as a line that failed to load.
            if (who.length === 0) {
                page.clearNames()
                return
            }
            page.namesText = page.namesPrefix + who.join(", ")
        }
    }

    // The face opens the reaction page in the mode that hands the character back.
    // It goes where the cursor is: an emoji belongs mid-sentence as often as at the end.
    function pickEmoji() {
        var dialog = pageStack.push(Qt.resolvedUrl("ReactionDialog.qml"),
                                    { forMessage: true })
        dialog.accepted.connect(function() {
            if (dialog.key.length > 0) {
                var position = messageField.cursorPosition
                messageField.text = messageField.text.slice(0, position)
                        + dialog.key + messageField.text.slice(position)
                messageField.cursorPosition = position + dialog.key.length
            }
            messageField.forceActiveFocus()
        })
    }

    /// The one line a message carries when its authenticity is in doubt. The
    /// reasons are the SDK's own codes; the wording is ours.
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

    // A tapped link. A Matrix address is answered inside the app; anything
    // else is a web address and goes where it always went.
    function followLink(link) {
        var target = MatrixLinks.parse(link)
        if (!target) {
            // Only the two schemes this app produces. Everything else the system would
            // dispatch is a stranger's choice, and the allowlist belongs at the last step.
            if (/^https?:\/\//i.test(link)) {
                // The address before it is opened: what a link says and where it
                // goes are two strings a stranger writes.
                var dialog = pageStack.push(Qt.resolvedUrl("ConfirmDialog.qml"), {
                                                question: qsTr("Open this address?"),
                                                subject: link,
                                                acceptLabel: qsTr("Open")
                                            })
                dialog.accepted.connect(function() { Qt.openUrlExternally(link) })
            }
            return
        }
        if (target.kind === "user") {
            // Not a silent direct chat: the dialog names the address and the
            // user accepts it.
            pageStack.push(Qt.resolvedUrl("NewChatDialog.qml"),
                           { prefill: target.id })
            return
        }
        page.pendingAddress = target.id
        matrix.resolveRoom(target.id)
    }

    // The address a tapped link asked about, until the core has answered.
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
            // Never from the link itself: joining is the user's decision, and
            // the dialog is where it is made.
            pageStack.push(Qt.resolvedUrl("JoinRoomDialog.qml"),
                           { prefill: address })
        }

        onRoomResolveFailed: {
            if (page.pendingAddress.length === 0) {
                return
            }
            page.pendingAddress = ""
        }
    }

    /// Everything a sender wrote loses its meaning as markup here, and only
    /// here. Whatever is appended afterwards was written by this app.
    function escapeBody(body) {
        return body.replace(/&/g, "&amp;")
                   .replace(/</g, "&lt;")
                   .replace(/>/g, "&gt;")
    }

    /// A URL inside an href, where Qt decodes no entities: the ampersand has to
    /// stay itself. Anything that could end the attribute is refused.
    function safeHref(url) {
        return /["'<>`\\\s]/.test(url) ? "" : url
    }

    function linkifyBody(body) {
        var escaped = page.escapeBody(body)
        // One pass over both kinds, web address first: a matrix.to permalink carries
        // a room address inside itself.
        var pattern = /(https?:\/\/[^\s<>"]+)|([#@!][A-Za-z0-9._=\-\/+]+:[A-Za-z0-9.\-]+(?::[0-9]+)?)/g
        return escaped.replace(pattern, function(match, url, address) {
            var trail = ""
            var target = url || address
            // Sentence punctuation glued to the end is not part of the link.
            var punctuation = target.match(/[.,;:!?]+$/)
            if (punctuation) {
                trail = punctuation[0]
                target = target.slice(0, target.length - trail.length)
            }
            if (url) {
                // A closing parenthesis only when none was opened.
                if (target.charAt(target.length - 1) === ")" && target.indexOf("(") < 0) {
                    target = target.slice(0, target.length - 1)
                    trail = ")" + trail
                }
                // A Matrix permalink is handled in the app, so it stays tappable even where
                // web links are off - it never reaches a browser.
                if (!settings.clickableLinks && !MatrixLinks.parse(target)) {
                    return target + trail
                }
                // The href as written, not as shown: `target` left the escaper with
                // `&amp;`, and Qt decodes no entities in an attribute.
                var href = page.safeHref(target.replace(/&amp;/g, "&"))
                if (href.length === 0) {
                    return target + trail
                }
                return "<a href=\"" + href + "\">" + target + "</a>" + trail
            }
            return "<a href=\"xmatic:" + target + "\">" + target + "</a>" + trail
        })
    }

    function storeFile(path, item) {
        if (item.msgtype === "m.image") {
            matrix.saveToPictures(path, item.media ? item.media.filename : "")
        } else {
            matrix.saveToDownloads(path, item.media ? item.media.filename : "")
        }
    }

    function openImage(itemId, media) {
        // A separate cache key: the timeline holds the thumbnail under the
        // plain item id, and both are worth keeping.
        var key = itemId + "/full"
        var known = matrix.mediaPath(key)
        matrix.requestMedia(key, media.source, false, media.size || 0)
        pageStack.push(Qt.resolvedUrl("ImageViewPage.qml"), {
                           mediaKey: key,
                           source: known.length > 0 ? "file://" + known : "",
                           fileName: media.filename || "",
                           mimeType: media.mimetype || "image/jpeg"
                       })
    }

    function beginEdit(eventId, body) {
        page.editingEventId = eventId
        messageField.text = body
        messageField.forceActiveFocus()
    }

    function cancelEdit() {
        page.editingEventId = ""
        messageField.text = ""
        messageField.focus = false
    }

    // What the notice below is currently saying.
    property string noticeText: ""

    /// A short word over the conversation, where a tap does nothing on purpose:
    /// silence reads as a tap that never registered.
    function showNotice(text) {
        page.noticeText = text
        jumpNoticeTimer.restart()
    }

    // Said out loud when a jump gives up.
    Rectangle {
        id: jumpNotice

        anchors {
            horizontalCenter: parent.horizontalCenter
            bottom: parent.bottom
            bottomMargin: Theme.itemSizeLarge
        }
        width: jumpNoticeLabel.width + 2 * Theme.paddingLarge
        height: jumpNoticeLabel.height + 2 * Theme.paddingMedium
        radius: Theme.paddingMedium
        color: Theme.rgba(Theme.highlightDimmerColor, 0.9)
        opacity: 0

        Label {
            id: jumpNoticeLabel

            anchors.centerIn: parent
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.primaryColor
            text: page.noticeText
        }

        Behavior on opacity { FadeAnimation { } }
    }

    Timer {
        id: jumpNoticeTimer

        interval: 2500
        onTriggered: jumpNotice.opacity = 0
        onRunningChanged: {
            if (running) {
                jumpNotice.opacity = 1
            }
        }
    }
}
