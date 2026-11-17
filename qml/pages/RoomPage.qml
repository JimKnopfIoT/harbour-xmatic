import QtQuick 2.5
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import QtMultimedia 5.6
import "Formatting.js" as Formatting
import "LinkMarks.js" as LinkMarks
import "MessageBody.js" as MessageBody
import "Strings.js" as Strings
import "Composing.js" as Composing
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

    /// What a row starts besides the composer; MessageRow and MessageActionsPage use it.
    property alias timelineActions: timelineActionsItem

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
    // Bound once here, not per row: the getter reads the settings file.
    readonly property string messageLayout: settings.messageLayout
    // No bubble: picture and name as a header, the text from margin to margin.
    readonly property bool flatLayout: messageLayout === "flat"
    // The same for a system line, where the picture only marks who acted.
    readonly property real systemAvatarSize: Theme.iconSizeExtraSmall

    // Which room events get a line; the switches are on the behaviour page.
    function systemShown(kind) {
        return timelineActionsItem.systemShown(kind)
    }

    /// A mention from the member page lands on the topmost page of the room that has a composer.
    readonly property bool takesMentions: !invited

    // Whether new messages should scroll the view along.
    property bool followTail: true

    // The timeline's height before the last change: keyboard, banner and composer
    // all take height away, and what was on screen has to stay there.
    property real lastTimelineHeight: 0
    /// Whether the reader stood at the newest message when the viewport began to
    /// change. Asked before the keyboard shortens the page, answered after.
    property bool wasAtEnd: false

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
    // A jump may open a slice: search hit yes, restored position no.
    property bool jumpMayFocus: true
    // Target time, where known: far beyond the loaded history opens a slice.
    property double jumpTimestamp: 0
    // A slice around one message instead of the live conversation. Empty: live.
    property string focusedEventId: ""
    // Writing actions return to live first.
    readonly property bool inSlice: focusedEventId.length > 0
    /// The thread page answers true; a row then offers no thread of its own.
    readonly property bool inThread: false
    // Set by "Back to the latest messages": open at the end.
    property bool returningToEnd: false

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
                            page.editingEventId.length > 0 ? "" : messageComposer.text)
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
            messageComposer.text = matrix.draft(roomId)
            matrix.openRoom(roomId)
            refreshRecipients(true)
            // The lock has to state the room's answer, not the caller's guess - not every
            // way in carries one, and the property defaults to encrypted.
            matrix.loadRoomInfo(roomId)
        } else if (roomName.length === 0) {
            // An invitation opens nothing: where the way in carried no name - a
            // notification - this answer is the only thing that fills the header.
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
    /// How many rounds the search may wait while the rows are still arriving. At
    /// the retry timer's 700 ms that is about seven seconds.
    property int unreadEmptyRounds: 0
    readonly property int unreadEmptyLimit: 10
    /// Set where the search gave up: the page then sits at the end without the
    /// reader having been there, and marking read would move the very marker the
    /// next attempt needs - the failure would delete its own evidence.
    property bool unreadJumpFailed: false
    /// The count the last round saw, so "still filling" can be told from "done".
    property int unreadLastCount: -1

    // Once per built timeline: coming back from a sub-page must not jump. A
    // rebuilt timeline is a different matter - the rows are new.
    property bool unreadHandled: false

    Connections {
        target: matrix

        // What the room says about itself against what the caller believed: encryption,
        // and the name where a notification tap arrived with the id alone.
        onRoomInfoReady: {
            if (info.roomId !== page.roomId) {
                return
            }
            // An invitation is stripped state: "not encrypted" read off it would open
            // the padlock and take the recipient warning with it. The name is safe.
            if (!page.invited) {
                page.encrypted = info.encrypted === true
                page.encryptionKnown = true
            }
            if (page.roomName.length === 0 && info.name.length > 0) {
                page.roomName = info.name
            }
        }

        onTimelineOpened: {
            // The live timeline is back, which is what a pending jump waited for. It goes
            // first: the message the user asked for beats where reading stopped.
            if (page.jumpTargetId.length > 0) {
                page.tryJump()
            }
            if (page.returningToEnd) {
                page.returningToEnd = false
                page.unreadHandled = true
                page.followTail = true
                timelineView.positionViewAtEnd()
                return
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
            // Where the reader stood when this room was last left, for as long as the app
            // runs. Only ever set away from the end, so the unread search keeps that case.
            var kept = positions.recall(page.roomId)
            if (kept.eventId) {
                // The line still has to say where the new messages start, and nothing
                // else will look for it on this path: where the marker names no event,
                // the receipt does. The unread search does the same, one step later.
                if (page.markerEventId.length === 0) {
                    page.markerEventId = page.unreadFallbackId
                }
                page.jumpToEvent(kept.eventId, ListView.Beginning, kept.offset, 0, false)
                // The live view is back - that is what this signal says - but the page
                // itself may still be coming up, and `jumpToEvent` waits for `Active`.
                page.tryJump()
                return
            }
            page.unreadFromId = readMarker
            page.unreadPagesLeft = page.unreadPageLimit
            page.unreadEmptyRounds = page.unreadEmptyLimit
            page.unreadLastCount = -1
            page.unreadJumpFailed = false
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
    // The member page has no composer; it hands the mention to the room page,
    // which is the one that stays. Applied when this page is on top again -
    // a focus set while the stack animates does not stick.
    Connections {
        target: matrix.mentions

        onInsertRequested: {
            if (roomId !== page.roomId || page.invited
                    || Composing.mentionHost(pageStack, roomId) !== page) {
                return
            }
            // The composer lives in the live conversation only.
            if (page.inSlice) {
                page.backToLive()
            }
            if (page.status === PageStatus.Active) {
                messageComposer.insertMember(userId, displayName)
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
        // The block from a failed jump only applies where there is something to
        // scroll: a room that fits on one screen was seen whole, and holding it
        // unread would be the reported complaint at a new place.
        if (invited || page.unreadFromId.length > 0
                || (page.unreadJumpFailed
                    && timelineView.contentHeight > timelineView.height)) {
            return
        }
        // `atYEnd` is false while the list still coasts after a flick, so leaving
        // right after scrolling to the newest message left the room unread - the
        // second of waiting the field reported. The last half screen counts.
        if (page.followTail || timelineView.atYEnd || page.nearTail) {
            matrix.markRead()
        }
    }

    readonly property bool nearTail:
        timelineView.contentHeight > 0
        && timelineView.contentY + timelineView.height
           >= timelineView.contentHeight - timelineView.height / 2

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
                page.stayAtEnd(true)
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
        // The core answers `timeline.open` before the rows arrive, so the first look
        // is too early. Wait while the model is still filling - "empty" was the wrong
        // measure: one row ends it while the marker is still on its way, and a small
        // room is then out of pages before it began. Bounded, so a room that never
        // delivers cannot hold the view.
        if (page.unreadEmptyRounds > 0
                && matrix.timeline.count !== page.unreadLastCount) {
            page.unreadLastCount = matrix.timeline.count
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
        page.stayAtEnd(false)
    }

    // The end, and the newest rows followed again. `read` is false where the jump
    // failed: marking read there moves the anchor the next attempt needs, so the
    // failure would delete its own evidence and the room never jumps again.
    function stayAtEnd(read) {
        page.followTail = true
        page.unreadJumpFailed = !read
        timelineView.positionViewAtEnd()
        if (read) {
            readTimer.restart()
        }
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
            // A hands-free take has no finger that lifts on leaving: dropped, not sent.
            if (matrix.recorder.handsFree) {
                matrix.recorder.cancel()
            }
            if (status === PageStatus.Deactivating) {
                page.keepAnchor()
                page.keepReadingPosition()
            }
        }

        // The page is on screen during the transition already, so this cannot wait
        // for `Active`: what is painted there is what the reader sees jump.
        if (status === PageStatus.Activating || status === PageStatus.Active) {
            page.restoreAnchor()
        }
        // Let go a moment after the return - held for ever, the anchor would
        // refuse every scroll.
        if (status === PageStatus.Active && page.keptIndex >= 0) {
            anchorHold.restart()
        }

        if (status === PageStatus.Active && !invited) {
            // Coming back from the pinned view, the shared timeline has to show live
            // events again - or the slice this page was showing. Otherwise a no-op.
            if (page.focusedEventId.length === 0) {
                matrix.openRoom(roomId)
            } else if (matrix.timelineFocus !== page.focusedEventId) {
                matrix.openRoom(roomId, page.focusedEventId)
            }
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
                } else if (action.kind === "otherFiles") {
                    pageStack.push(multiContentPicker)
                } else if (action.kind === "textFile") {
                    sendTextFile()
                } else if (action.kind === "react") {
                    // These three push a page of their own. From the actions page, after its pop
                    // had started, `push()` handed nothing back and the picker never appeared.
                    timelineActionsItem.pickReaction(action.eventId)
                } else if (action.kind === "thread") {
                    timelineActionsItem.openThread(action.item)
                } else if (action.kind === "forwardAttachment") {
                    timelineActionsItem.forwardAttachment(action.item)
                } else if (action.kind === "mention") {
                    messageComposer.insertMember(action.userId, action.displayName)
                } else if (action.kind === "transcribe") {
                    matrix.transcripts.transcribe(action.item.id, action.eventId, action.item.media)
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

    // Where the reader stood, as a row and a distance into it. A pixel position
    // means nothing across a page change: covered, the list destroys its
    // delegates, and on return it re-estimates. Measured on the device, four
    // times over: contentHeight 31135 -> 42134 with an unchanged count of 50,
    // originY moving with it, and the kept contentY then twelve rows too high.
    property int keptIndex: -1
    property real keptOffset: 0

    /// Takes the anchor. At the end there is nothing to keep - the tail logic
    /// owns that case and puts the view back exactly.
    function keepAnchor() {
        page.keptIndex = -1
        if (page.followTail || timelineView.atYEnd
                || page.jumpTargetId.length > 0 || page.unreadFromId.length > 0) {
            return
        }
        var probeY = timelineView.contentY + 1
        var idx = timelineView.indexAt(timelineView.width / 2, probeY)
        if (idx < 0) {
            return
        }
        var item = timelineView.itemAt(timelineView.width / 2, probeY)
        page.keptIndex = idx
        page.keptOffset = item ? (timelineView.contentY - item.y) : 0
    }

    property bool restoringAnchor: false

    /// Puts the row back under the top edge. Re-applied while the list is still
    /// re-measuring, because one application lands on the geometry of that frame.
    function restoreAnchor() {
        // Positioning creates delegates, which changes the content height, which is
        // where this is called from: without the guard it calls itself.
        if (page.keptIndex < 0 || page.restoringAnchor) {
            return
        }
        // A hand on the list outranks the anchor, and so does a jump the user asked
        // for. Both end the restore rather than fight it.
        if (page.jumpTargetId.length > 0 || page.unreadFromId.length > 0
                || ((timelineView.moving || timelineView.dragging)
                    && page.status === PageStatus.Active)) {
            page.keptIndex = -1
            return
        }
        page.restoringAnchor = true
        timelineView.positionViewAtIndex(page.keptIndex, ListView.Beginning)
        timelineView.contentY += page.keptOffset
        page.restoringAnchor = false
    }

    // The re-measuring runs on for a while after the page is back, so the anchor
    // is held for a moment and then let go - a restore that never ends would
    // refuse every scroll.
    Timer {
        id: anchorHold

        interval: 900
        onTriggered: page.keptIndex = -1
    }

    // Asked once for the page: the plugin set does not change while it is open.
    readonly property var animatableFormats: matrix.animatableImageFormats()

    // Someone else's message, where the room's power levels allow it. `=== true`, not
    // the pin entry's `!== false`: an unanswered room offers nothing here.
    readonly property bool canRedactOthers: matrix.roomPermissions.redactOthers === true

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

    /// The row a restored reading position sits on, held by event id while the list
    /// re-estimates its height: one positioning lands on the geometry of that frame,
    /// and delegates coming into being move everything under them.
    property string holdEventId: ""
    property real holdOffset: 0

    function holdPosition() {
        if (page.holdEventId.length === 0 || page.restoringAnchor) {
            return
        }
        // A hand on the list outranks the hold, the same way it outranks the anchor.
        if ((timelineView.moving || timelineView.dragging)
                && page.status === PageStatus.Active) {
            page.holdEventId = ""
            return
        }
        // By id, never by index: a pagination inserts older rows above and every
        // index moves with it.
        var idx = matrix.timeline.indexOfEvent(page.holdEventId)
        if (idx < 0) {
            return
        }
        page.restoringAnchor = true
        timelineView.positionViewAtIndex(idx, ListView.Beginning)
        timelineView.contentY += page.holdOffset
        page.restoringAnchor = false
    }

    // Long enough to outlast the re-estimating, short enough not to refuse a scroll.
    Timer {
        id: holdRelease

        interval: 1500
        onTriggered: page.holdEventId = ""
    }

    /// Where a jump puts its target, and how far it is then shifted.
    property int jumpAlign: ListView.Center
    property real jumpOffset: 0

    /// The topmost visible row, kept by event id for as long as the app runs: a
    /// room left in the middle of its history opens there again. The end is not
    /// kept - a room opens there anyway, and the unread logic owns that case.
    function keepReadingPosition() {
        // A jump that has not landed yet is not a position; what is held stays held.
        // Nor is a place in a slice: coming back, the room opens live.
        if (page.invited || page.roomId.length === 0 || page.jumpTargetId.length > 0
                || page.focusedEventId.length > 0) {
            return
        }
        if (page.followTail || timelineView.atYEnd) {
            positions.forget(page.roomId)
            return
        }
        var probeY = timelineView.contentY + 1
        var item = timelineView.itemAt(timelineView.width / 2, probeY)
        // A row that is still on its way to the server has no id to come back to.
        if (!item || !item.rowEventId || item.rowEventId.length === 0) {
            return
        }
        positions.remember(page.roomId, item.rowEventId, timelineView.contentY - item.y)
    }

    // Scrolls to a message, loading older history until it shows up. Called by
    // the pinned overview before it pops back, and by tapping a quote.
    function jumpToEvent(eventId, align, offset, timestamp, mayFocus) {
        if (!eventId || eventId.length === 0) {
            return
        }
        jumpTimestamp = timestamp || 0
        jumpMayFocus = mayFocus !== false
        // A new jump beats a pending return to the end.
        returningToEnd = false
        // A quote lands in the middle of the view; a reading position goes back under
        // the top edge, where it stood.
        jumpAlign = align === undefined ? ListView.Center : align
        jumpOffset = offset === undefined ? 0 : offset
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
            // Slice not built: say so; the button leads back.
            if (inSlice) {
                showNotice(qsTr("That message is not in the loaded history"))
            }
            return
        }
        var idx = matrix.timeline.indexOfEvent(jumpTargetId)
        if (idx >= 0) {
            var landed = jumpTargetId
            jumpTargetId = ""
            jumpRetry.stop()
            followTail = false
            timelineView.positionViewAtIndex(idx, jumpAlign)
            timelineView.contentY += jumpOffset
            // A restored reading position is held for a moment; a jump the user asked
            // for is a single act and keeps none.
            if (jumpAlign === ListView.Beginning) {
                page.holdEventId = landed
                page.holdOffset = jumpOffset
                holdRelease.restart()
            }
            return
        }
        // Far beyond the loaded history: one slice instead of many pages.
        var oldest = matrix.timeline.oldestTimestamp()
        if (jumpMayFocus && jumpTimestamp > 0 && oldest > 0
                && jumpTimestamp < oldest - 24 * 3600 * 1000) {
            openFocused(jumpTargetId)
            return
        }
        // From a slice: the next slice, no paging.
        if (jumpMayFocus && inSlice) {
            openFocused(jumpTargetId)
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
        var missed = jumpTargetId
        jumpTargetId = ""
        jumpRetry.stop()
        // Pages ran out: a slice, once per target.
        if (jumpMayFocus && missed !== focusedEventId) {
            openFocused(missed)
            return
        }
        showNotice(qsTr("That message is not in the loaded history"))
    }

    // The message in a slice; the jump waits for the model as for live.
    function openFocused(eventId) {
        focusedEventId = eventId
        followTail = false
        jumpTargetId = eventId
        jumpAlign = ListView.Center
        jumpOffset = 0
        jumpPagesLeft = 0
        jumpWaitsLeft = jumpWaitLimit
        jumpMayFocus = false
        matrix.openRoom(roomId, eventId)
        jumpRetry.restart()
    }

    // Back from the slice to the conversation as it is now.
    function backToLive() {
        // A kept row index belongs to the slice.
        keptIndex = -1
        focusedEventId = ""
        jumpTargetId = ""
        jumpRetry.stop()
        followTail = true
        returningToEnd = true
        positions.forget(roomId)
        matrix.openRoom(roomId)
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

    // A slice reads on towards the live edge once its end is reached by hand.
    // Not from the tail logic: following new rows to the end would page it all.
    property int newerEmptyRounds: 0
    property real newerLastHeight: -1

    function loadNewerAtEnd(byHand) {
        if (!page.inSlice || !matrix.timelineReady || matrix.paginating
                || matrix.timelineAtEnd || timelineView.count === 0) {
            return
        }
        if (byHand) {
            page.newerEmptyRounds = 0
        } else if (!timelineView.atYEnd
                   || timelineView.contentHeight > page.newerLastHeight
                   || ++page.newerEmptyRounds >= page.fillEmptyLimit) {
            // After an answer, again only where the page added nothing to see.
            return
        }
        page.newerLastHeight = timelineView.contentHeight
        matrix.loadNewer()
    }

    Connections {
        target: matrix

        // The round is over, and where it brought nothing no diff will arrive to
        // trigger the fill. Not `busyChanged`: a downloading avatar drove history.
        onPaginated: {
            page.fillScreen()
            // A slice shorter than the screen cannot be scrolled to its end by hand.
            if (page.inSlice && timelineView.contentHeight <= timelineView.height) {
                page.loadNewerAtEnd(true)
            }
        }
        onPaginatedNewer: page.loadNewerAtEnd(false)

        // A different room, a fresh start.
        onTimelineReadyChanged: {
            page.fillEmptyRounds = 0
            page.fillLastHeight = -1
            page.newerEmptyRounds = 0
            page.newerLastHeight = -1
            page.fillScreen()
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

            // Made here rather than from the composer's attach button: a poll is
            // not an attachment, and the picker is about files.
            MenuItem {
                text: qsTr("New poll")
                visible: !page.invited
                onClicked: {
                    if (page.inSlice) {
                        page.backToLive()
                    }
                    pageStack.push(Qt.resolvedUrl("CreatePollDialog.qml"))
                }
            }

            // Only where Privacy allows sending the position at all.
            MenuItem {
                text: qsTr("Share location")
                visible: !page.invited && settings.locationSharing
                onClicked: {
                    if (page.inSlice) {
                        page.backToLive()
                    }
                    pageStack.push(Qt.resolvedUrl("ShareLocationPage.qml"), {
                                       roomId: page.roomId,
                                       encrypted: page.encrypted || !page.encryptionKnown
                                   })
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

        SendQueueBanner {
            id: sendQueueBanner

            anchors {
                left: parent.left
                right: parent.right
                top: pinnedBanner.bottom
            }
            visible: !page.invited && head !== null
        }

        SilicaListView {
            id: timelineView

            anchors {
                left: parent.left
                right: parent.right
                top: sendQueueBanner.bottom
                bottom: composer.top
            }
            clip: true
            // The model belongs to whichever room the core has open, and an
            // invitation never opens one - bound, it shows the room visited before.
            model: page.invited ? [] : matrix.timeline
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

            delegate: MessageRow {
                host: page
                listView: timelineView
                player: audioPlayer
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

            // The run of height changes is over once none has come for a moment. A
            // reader who stood at the end is put back there exactly, not arithmetically.
            Timer {
                id: viewportSettle

                interval: 120
                onTriggered: {
                    if (page.wasAtEnd && page.jumpTargetId.length === 0
                            && page.holdEventId.length === 0) {
                        timelineView.positionViewAtEnd()
                        page.followTail = true
                    }
                }
            }

            onMovementEnded: {
                page.followTail = atYEnd
                if (atYEnd) {
                    page.loadNewerAtEnd(true)
                }
                // A hand on the list outranks the marker search: being carried off to a jump
                // one has just scrolled away from is worse than not being taken there.
                page.unreadFromId = ""
                unreadRetry.stop()
                // Moved by hand, so where the list stands is where the reader put it -
                // the block a failed jump left behind has done its job.
                page.unreadJumpFailed = false
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
                // The estimate moved: whatever the anchor was pointing at is
                // somewhere else now, so it is applied again. Deliberately not
                // only while the page is on top - the re-measuring happens while
                // it is covered, and correcting it afterwards is a visible jump.
                if (page.keptIndex >= 0) {
                    page.restoreAnchor()
                }
                page.holdPosition()
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
                // A height of nothing is a moment during layout, not a viewport the reader
                // lost: measured, the list is briefly negative in height while the composer
                // is laid out, and acting on that pins it to its first row.
                if (height <= 0 || lost === 0) {
                    return
                }
                // Not while the room is still deciding where to open. The unread
                // marker positions the list itself, and `followTail` is still at its
                // default `true` then - the settle below would answer "the reader
                // stood at the end" and undo the very jump the room was opened for.
                if (page.status !== PageStatus.Active
                        || page.jumpTargetId.length > 0
                        || page.holdEventId.length > 0
                        || page.unreadFromId.length > 0
                        || unreadRetry.running || followCheck.running) {
                    return
                }
                // The keyboard arrives over many frames. Where the reader stood is asked
                // once, at the first of them, and answered again when they stop coming:
                // this list's content height is an estimate that halves and doubles as
                // delegates come and go, so holding contentY does not hold the position.
                if (!viewportSettle.running) {
                    page.wasAtEnd = page.followTail || atYEnd
                }
                viewportSettle.restart()
                if (lost > 0 && contentHeight > height) {
                    contentY = Math.min(contentY + lost,
                                        originY + contentHeight - height)
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

        // Media, reactions and the names and shield lines: shared with the thread.
        TimelineActions {
            id: timelineActionsItem

            page: page
            player: audioPlayer
            timelineModel: matrix.timeline
        }

        // One player for the page: two voice messages never play at once, and the
        // delegate is recycled while scrolling.
        Audio {
            id: audioPlayer

            onStopped: {
                if (status === Audio.EndOfMedia) {
                    timelineActionsItem.playingId = ""
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
                    : page.focusedEventId.length > 0
                    ? liveButton.height + 2 * Theme.paddingLarge
                    : composerColumn.height + Theme.paddingSmall

            Button {
                id: joinButton

                visible: page.invited
                anchors.centerIn: parent
                text: qsTr("Accept invitation")
                onClicked: {
                    matrix.joinRoom(page.roomId)
                    // Open first: it empties the model, so the list never binds to
                    // the previous room's rows.
                    matrix.openRoom(page.roomId)
                    page.invited = false
                    // The invitation's encryption answer came off stripped state: back
                    // to the default, which counts as encrypted where that is safer.
                    page.encrypted = true
                    // Not through refreshRecipients: no status change follows, so
                    // nothing else asks - and its stamp would throttle the real check.
                    matrix.checkRecipients(page.roomId)
                }
            }

            // No sending in a slice: the send would take its thread.
            Button {
                id: liveButton

                visible: !page.invited && page.focusedEventId.length > 0
                anchors.centerIn: parent
                text: qsTr("Back to the latest messages")
                onClicked: page.backToLive()
            }

            Column {
                id: composerColumn

                visible: !page.invited && page.focusedEventId.length === 0
                anchors {
                    left: parent.left
                    right: parent.right
                    bottom: parent.bottom
                }

                // While recording, the elapsed time replaces the hints above the
                // field so it is obvious that the microphone is live.
                Item {
                    width: parent.width
                    height: recordingLabel.visible
                            ? Math.max(recordingLabel.height,
                                       cancelRecording.visible ? cancelRecording.height : 0)
                            : 0

                    Label {
                        id: recordingLabel

                        x: Theme.horizontalPageMargin
                        width: parent.width - 2 * Theme.horizontalPageMargin
                               - (cancelRecording.visible ? cancelRecording.width : 0)
                        anchors.verticalCenter: parent.verticalCenter
                        visible: matrix.recorder.recording
                        wrapMode: Text.Wrap
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.errorColor
                        text: matrix.recorder.handsFree
                              ? qsTr("Recording… %1 s. Tap the microphone to send.")
                                    .arg(Math.round(matrix.recorder.duration / 1000))
                              : qsTr("Recording… %1 s").arg(Math.round(matrix.recorder.duration / 1000))
                    }

                    // Hands-free has no finger to lift: this is its way out without sending.
                    IconButton {
                        id: cancelRecording

                        anchors {
                            right: parent.right
                            verticalCenter: parent.verticalCenter
                        }
                        visible: matrix.recorder.handsFree
                        icon.source: "image://theme/icon-m-clear"
                        onClicked: matrix.recorder.cancel()
                    }
                }

                Row {
                    x: Theme.horizontalPageMargin
                    width: parent.width - Theme.horizontalPageMargin
                    visible: page.replyingEventId.length > 0
                    spacing: Theme.paddingSmall

                    Label {
                        width: parent.width - parent.spacing - cancelReply.width
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
                    x: Theme.horizontalPageMargin
                    width: parent.width - Theme.horizontalPageMargin
                    visible: page.editingEventId.length > 0
                    spacing: Theme.paddingSmall

                    Label {
                        width: parent.width - parent.spacing - cancelEdit.width
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

                Composer {
                    id: messageComposer

                    // Anchored, not bound: a width that is briefly zero makes the
                    // field's implicit height enormous, and the list under it loses
                    // its viewport for a frame.
                    anchors {
                        left: parent.left
                        right: parent.right
                    }
                    roomId: page.roomId
                    attachments: true
                    emoji: true
                    voice: true
                    editing: page.editingEventId.length > 0
                    placeholderText: page.editingEventId.length > 0
                                     ? qsTr("New text") : qsTr("Message")

                    onSubmitted: page.submit()
                    onAttachRequested: pageStack.push(picturePicker)
                    onTextFileRequested: page.sendTextFile()
                    onEmojiRequested: page.pickEmoji()
                    onRecordingStopped: page.followTail = true
                }
            }
        }
    }


    // Gallery by folder or the file system, in one page. The platform offers
    // each of them but not together, and its gallery is one undivided grid.
    Component {
        id: picturePicker

        AttachmentPickerPage {
            // Only whether there is one: the text stays on this page.
            draftText: messageComposer.text.trim().length > 0

            onAccepted: {
                // Handed to the room, never pushed from here: a dialog that is
                // popping does not hand back the page it would push.
                if (textFileWanted) {
                    page.pendingAction = { "kind": "textFile" }
                    return
                }
                if (otherFilesWanted) {
                    page.pendingAction = { "kind": "otherFiles" }
                    return
                }
                if (picked.length > 0) {
                    page.pendingAttachment = picked
                    attachmentWait.restart()
                }
            }
        }
    }

    // Several files in one go. Matrix has no album: each one is its own event,
    // and the caption belongs to the first of them.
    Component {
        id: multiContentPicker

        MultiContentPickerDialog {
            title: qsTr("Select files")

            onAccepted: {
                var picked = []
                for (var i = 0; i < selectedContent.count; i++) {
                    var item = selectedContent.get(i)
                    picked.push({ path: item.filePath, mimeType: item.mimeType })
                }
                if (picked.length === 0) {
                    return
                }
                page.pendingAttachment = picked
                attachmentWait.restart()
            }
        }
    }

    // The files picked, until the conversation is back on top and the send page
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
                var typed = messageComposer.text
                var typedMentions = messageComposer.mentionsPicked
                messageComposer.clearField()
                messageComposer.clearMentions()
                pageStack.push(Qt.resolvedUrl("SendMediaPage.qml"), {
                    files: picked,
                    caption: typed,
                    roomId: page.roomId,
                    mentionsPicked: typedMentions,
                    replyTo: page.replyingEventId,
                    replySender: page.replyingTo,
                    replyBody: page.replyingBody,
                    // Handed back here rather than sent from the dialog: the unverified-recipient
                    // warning lives on this page, and an attachment went past it.
                    send: function (files, caption, replyTo, original, mentions) {
                        page.pendingAction = {
                            "kind": "sendMedia",
                            "files": files,
                            "caption": caption,
                            "mentions": mentions,
                            "replyTo": replyTo,
                            "original": original
                        }
                    },
                    afterCancel: function (text, mentions) {
                        messageComposer.text = text
                        messageComposer.mentionsPicked = mentions
                        // A photo taken for this send goes with it; gallery files are not ours.
                        for (var i = 0; i < picked.length; i++) {
                            matrix.cameraShots.discard(picked[i].path)
                        }
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

    function submit() {
        // The word being typed is in the input method's preedit, not in `text`: whoever
        // reaches straight for send sent everything but the last word.
        Qt.inputMethod.commit()
        // In an encrypted room, warn once before a message reaches a recipient with
        // unverified devices. The dialog handles "send anyway" and calls back.
        var pending = Composing.pendingUnverified(page.unverifiedUsers, matrix)
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
            messageComposer.releaseField()
            return
        }
        // Keeping the keyboard is an action, not the absence of one: the arrow
        // that was just tapped is a press outside the field.
        messageComposer.holdKeyboard()
        messageComposer.focusField()
    }

    function doSubmit() {
        if (page.editingEventId.length > 0) {
            // Those picked while editing; the core keeps the original's still named.
            matrix.editMessage(page.editingEventId, messageComposer.text,
                               messageComposer.mentionIds())
            // cancelEdit drops the focus by itself - an edit that is done is
            // done, whatever the setting says.
            page.cancelEdit()
            return
        }
        var mentions = messageComposer.mentionIds()
        if (page.replyingEventId.length > 0) {
            matrix.replyToMessage(page.replyingEventId, messageComposer.text, mentions)
            messageComposer.clearMentions()
            page.cancelReply()
            page.afterSend()
            return
        }
        matrix.sendMessage(messageComposer.text, mentions)
        messageComposer.clearField()
        messageComposer.clearMentions()
        page.afterSend()
    }

    /// The draft as a .txt attachment, asked for in the attachment picker. Through
    /// the same gate a typed message goes through: a file reaches the same recipients.
    function sendTextFile() {
        Qt.inputMethod.commit()
        if (messageComposer.text.trim().length === 0) {
            return
        }
        var pending = Composing.pendingUnverified(page.unverifiedUsers, matrix)
        if (pending.length > 0) {
            var dialog = pageStack.push(Qt.resolvedUrl("UnverifiedRecipientsDialog.qml"),
                                        { users: pending })
            dialog.accepted.connect(page.doSendTextFile)
            return
        }
        page.doSendTextFile()
    }

    function doSendTextFile() {
        if (!matrix.sendTextAsFile(messageComposer.text, page.replyingEventId)) {
            page.showNotice(qsTr("The text could not be written as a file"))
            return
        }
        messageComposer.clearField()
        messageComposer.clearMentions()
        page.clearReplyState()
        page.afterSend()
    }

    /// Long messages start folded; this remembers which are open. Keyed by row id,
    /// since a delegate that scrolls away forgets.
    property var expandedRows: ({})
    property int expandedRevision: 0

    function toggleExpanded(rowId) {
        if (page.expandedRows[rowId]) {
            delete page.expandedRows[rowId]
        } else {
            page.expandedRows[rowId] = true
        }
        page.expandedRevision++
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
        var pending = Composing.pendingUnverified(page.unverifiedUsers, matrix)
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
        // One event per file, in the order they were picked. Caption and reply
        // belong to the first: a reply answers one event, not a batch.
        for (var i = 0; i < action.files.length; i++) {
            var file = action.files[i]
            matrix.sendMedia(file.path, file.mimeType,
                             i === 0 ? action.caption : "",
                             i === 0 ? action.replyTo : "",
                             0, action.original === true,
                             i === 0 && action.mentions ? action.mentions : [])
        }
        page.clearReplyState()
        page.followTail = true
    }

    function beginReply(eventId, sender, body) {
        // Written from live: a slice would take the thread.
        if (inSlice) {
            backToLive()
        }
        page.replyingEventId = eventId
        page.replyingTo = sender && sender.length > 0 ? sender : body
        // Kept separately from replyingTo: the send page shows sender and text
        // as two lines, the way the quote in the conversation does.
        page.replyingBody = body || ""
        page.editingEventId = ""
        messageComposer.focusField()
    }

    function cancelReply() {
        page.replyingEventId = ""
        page.replyingTo = ""
        page.replyingBody = ""
        messageComposer.clearField()
    }

    // After an attachment went out as a reply. Unlike `cancelReply` this leaves
    // the composer alone - a half-typed message is not part of what was sent.
    function clearReplyState() {
        page.replyingEventId = ""
        page.replyingTo = ""
        page.replyingBody = ""
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

    // The face opens the reaction page in the mode that hands the character back.
    // It goes where the cursor is: an emoji belongs mid-sentence as often as at the end.
    function pickEmoji() {
        var dialog = pageStack.push(Qt.resolvedUrl("ReactionDialog.qml"),
                                    { forMessage: true })
        dialog.accepted.connect(function() {
            if (dialog.key.length > 0) {
                messageComposer.insertAtCursor(dialog.key)
            }
            messageComposer.focusField()
        })
    }

    // A tapped link, shared with the thread: `TimelineActions.followLink`.
    function followLink(link, map) {
        timelineActionsItem.followLink(link, map)
    }

    function beginEdit(eventId, body) {
        page.editingEventId = eventId
        messageComposer.clearMentions()
        messageComposer.text = body
        messageComposer.focusField()
    }

    function cancelEdit() {
        page.editingEventId = ""
        messageComposer.clearField()
        messageComposer.clearMentions()
        messageComposer.releaseField()
    }

    // What the notice below is currently saying.
    property string noticeText: ""

    /// A short word over the conversation, where a tap does nothing on purpose:
    /// silence reads as a tap that never registered.
    function showNotice(text) {
        page.noticeText = text
        jumpNoticeTimer.restart()
    }

    // A hands-free take that heard nothing is dropped, and the page says so.
    Connections {
        target: matrix.recorder
        onNothingHeard: page.showNotice(qsTr("Nothing was said, so nothing was sent."))
    }

    Connections {
        target: matrix.roomSettings
        onSaved: {
            if (roomId === page.roomId && shownName.length > 0) {
                page.roomName = shownName
            }
        }
    }

    // A refused poll command has no row to mark, so the page says it.
    Connections {
        target: matrix.polls
        onFailed: {
            if (command === "poll.vote") {
                page.showNotice(qsTr("Voting failed"))
            } else if (command === "poll.end") {
                page.showNotice(qsTr("The poll could not be ended"))
            } else {
                page.showNotice(qsTr("The poll could not be sent"))
            }
        }
        onVoteFailed: page.showNotice(qsTr("Your vote was not sent"))
    }

    Connections {
        target: matrix.sendQueue
        // "No longer queued": sent or removed, not a failure.
        onFailed: page.showNotice(message.indexOf("no longer queued") >= 0
                                  ? qsTr("It is no longer waiting to be sent.")
                                  : qsTr("That did not work. Try again in a moment."))
    }

    // Start failed or unanswered: stopped, and said.
    Connections {
        target: matrix.locations
        onLiveFailed: {
            if (roomId === page.roomId) {
                page.showNotice(qsTr("Live location could not be started"))
            }
        }
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
