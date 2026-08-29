import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import QtMultimedia 5.6
import "Formatting.js" as Formatting
import "MatrixLinks.js" as MatrixLinks

// A single room: history above, composer below.
//
// The list keeps the core's order — oldest first — and follows the bottom
// unless the user has scrolled up to read.
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
    // Whether that value came from the room itself rather than from whoever
    // pushed this page. The padlock is the only thing that still states the
    // room's encryption, and half the ways in here (a notification, the D-Bus
    // start, a tapped room link) carry nothing - while the property defaults to
    // "encrypted". A security mark whose failure state is "safe" is worse than
    // none, so it says nothing until the room has answered.
    property bool encryptionKnown: false

    // Media keys whose fetch came back as a failure - refused for size, dropped
    // by the network, given up on by the watchdog. Without them the row cannot
    // tell "still loading" from "never coming" and turns its indicator for good.
    property var failedMedia: ({})

    // Set while an already sent message is being rewritten.
    property string editingEventId: ""
    // Set while a reply is being composed.
    property string replyingEventId: ""
    /// The quoted message's text, for the quote on the send page.
    property string replyingBody: ""
    property string replyingTo: ""

    allowedOrientations: Orientation.All

    // The sender's picture in the conversation, and the gap it leaves in front
    // of the bubble. The same size the chat list gives a room - reported as too
    // small next to it, and the two are looked at one after the other. One
    // number for all three places: the picture, the bubble's left margin and
    // the width the text may take. Never read off the item itself, which is the
    // width rule this delegate has already paid for three times.
    readonly property real avatarSize: Theme.iconSizeMedium

    // Whether new messages should scroll the view along.
    property bool followTail: true

    // The timeline's own height, as it was before the last change. The
    // keyboard, the pinned banner and a growing composer all take height away
    // from the list, and what was on screen has to stay on screen.
    property real lastTimelineHeight: 0

    // A save that is waiting for its download to finish.
    property string pendingSaveKey: ""

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

    // Recipients of this encrypted room that still have unverified devices,
    // filled by the core's answer to checkRecipients. Drives the pre-send
    // warning. Each entry is { userId, name, devices }.
    property var unverifiedUsers: []

    // Set while the join into the room that replaced this one is in flight.
    // Its own flag, not `busy`: that one is true for any command at all.
    property bool followingSuccessor: false

    // What the message-actions page chose, applied once this page is on top
    // again: reply and edit put the cursor in the composer, and focus set
    // while the stack is still animating does not stick.
    property var pendingAction: null

    // The way out is the way the draft is kept: this fires before the children
    // go, so the field can still be read. An edit in progress is not a draft -
    // its text belongs to a message that already exists - and neither is an
    // empty field, which removes whatever stood there before.
    Component.onDestruction: {
        if (!invited) {
            // Same reason as in submit(): without this the draft keeps
            // everything except the word the user stopped in the middle of.
            Qt.inputMethod.commit()
            matrix.setDraft(roomId,
                            page.editingEventId.length > 0 ? "" : messageField.text)
        }
    }

    // Leaving the foreground aborts a running countdown at once, instead of only
    // suppressing it when it expires. Suppressing at expiry was not enough:
    // minimising the app and coming back inside the four seconds left the
    // countdown running, and it fired on return. The remorse object has to be
    // kept for that - Remorse.popupAction() hands it back.
    property var activeRemorse: null
    readonly property bool appForeground: Qt.application.active
    onAppForegroundChanged: {
        if (!appForeground && activeRemorse && activeRemorse.pending) {
            activeRemorse.cancel()
        }
    }


    Component.onCompleted: {
        if (!invited) {
            // What was typed here last time and never sent. Going back to the
            // chat list destroys this page, so the field has to be filled from
            // somewhere that outlives it.
            messageField.text = matrix.draft(roomId)
            matrix.openRoom(roomId)
            refreshRecipients(true)
            // The lock at the top says whether this room is encrypted, so the
            // answer has to be the room's and not the caller's guess: not every
            // way in here carries one, and the property defaults to encrypted.
            // Read from local state, so it comes back at once.
            matrix.loadRoomInfo(roomId)
        }
    }

    // When the recipients were last asked about, so a return to this page does
    // not ask again straight away.
    property double lastRecipientCheck: 0
    readonly property int recipientCheckInterval: 60000

    // The warning only makes sense for an encrypted room; an unencrypted one
    // has no device trust to check. The core answers with recipientsChecked.
    //
    // Throttled, because this walks every member of the room and asks the
    // server about each one whose devices are not in the store yet. It used to
    // run on every `PageStatus.Active` - so closing the member list, an
    // attachment, a picture, or in landscape any message action at all paid for
    // a full pass through a five-hundred-member room. Devices do not change
    // that fast; a minute is closer to the truth than "every time a sub-page
    // pops".
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

    // How many pages of history the search for that row may still ask for.
    // It is hunted for after all: the first batch a room hands over is the
    // newest slice of the event cache, and the more there is unread the surer
    // the marker sits further back than that - which is exactly the room the
    // jump exists for. Giving up on the first look meant the feature worked
    // where it was not needed and failed where it was.
    property int unreadPagesLeft: 0
    readonly property int unreadPageLimit: 8
    /// How many rounds the search may wait for the first rows before it starts
    /// paginating. At the retry timer's 700 ms that is about seven seconds.
    property int unreadEmptyRounds: 0
    readonly property int unreadEmptyLimit: 10

    // Once per built timeline. The page stays alive while a sub-page is on top,
    // and coming back re-opens the same one - the position must not jump then.
    // A timeline that was actually rebuilt is a different matter: the rows are
    // new, the view starts from nothing, and where reading stopped is exactly
    // where it belongs. That is the way back from a room opened over this one.
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

        // What the room says about itself, as against what whoever pushed this
        // page believed. Only the encryption is taken: everything else on this
        // page comes from the timeline.
        onRoomInfoReady: {
            if (info.roomId === page.roomId) {
                page.encrypted = info.encrypted === true
                page.encryptionKnown = true
            }
        }

        onTimelineOpened: {
            // The live timeline is back - which is what a pending jump has
            // been waiting for. It goes first: where reading stopped matters
            // less than the message the user just asked to be taken to, and
            // showFirstUnread stands aside for exactly that reason.
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
            page.unreadFromId = readMarker
            page.unreadPagesLeft = page.unreadPageLimit
            page.unreadEmptyRounds = page.unreadEmptyLimit
            // The tail is let go of before the search starts, not when it
            // succeeds: a view that follows the newest row marks the room read
            // after eight hundred milliseconds, and that moves the very marker
            // this is looking for.
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

    // Sending the receipt is throttled, not because the core is slow but
    // because a burst of arriving rows would otherwise produce one command per
    // row. The SDK drops a receipt that is already covered, so the cost of a
    // late one is nil.
    Timer {
        id: readTimer

        interval: 800
        onTriggered: {
            // Only while the room is really being looked at, and only from the
            // tail: a view parked at the first unread message has not read what
            // is below it.
            //
            // `atYEnd` counts as much as the flag. The flag says the view is
            // *meant* to follow, and it is switched off by a jump, by the
            // opening at the first unread message and by a room that is
            // re-entered with its rows kept - after all of which the newest
            // message can perfectly well be on screen. Only the flag was asked,
            // and the room then stayed unread while it was being read, which is
            // how "it does not mark as read on its own" was reported. Leaving
            // the page has always asked both.
            //
            // But not while the search for the read marker is still running.
            // A room opens at its end, so `atYEnd` is true for the second or
            // two that search needs - and marking read there moves the very
            // marker it is looking for to the newest message. The next visit
            // then has nothing to jump to, which is exactly the report that
            // "opening at the last unread message does not work": it worked,
            // and asking `atYEnd` here took it apart again.
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

    // Leaving the page is not the only way to stop reading. The screen locks,
    // the home key goes, another app comes up - and the page stays Active
    // through all of it, so the branch in onStatusChanged never runs and the
    // receipt for everything just read was never sent. To the other side that
    // reads as "he never read it", which is exactly how it was reported.
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

    // Leaving is the last moment to say the room was read, and it has to be
    // taken on `Deactivating`.
    //
    // This used to hang on `Inactive` alone, which a page reaches when another
    // one covers it - not when it is popped. Swiping back destroys the page,
    // and it was gone before that branch ever ran: the receipt was dropped and
    // the room stood in the list as unread although every message had been on
    // screen. What hid the fault is the 800 ms debounce, which rescues anybody
    // who lingers a moment; measured on the device as "wait two or three
    // seconds and it works". Called from both states, and sending twice costs
    // nothing - the SDK drops a receipt that is already covered.
    //
    // Same guard as the timer: a room left while it is still looking for the
    // read marker has not been read to the end, whatever the view happens to
    // be showing.
    function markReadIfDue() {
        if (invited || page.unreadFromId.length > 0) {
            return
        }
        if (page.followTail || timelineView.atYEnd) {
            matrix.markRead()
        }
    }

    // Opens where this account stopped reading. The marker names the last read
    // message, so that message goes to the top of the screen: the line sits
    // under it and the first unread message under that, both in view. Putting
    // the first unread at the top instead left the line off the screen, and
    // nothing showed what had been read before it.
    //
    // Where the row is not loaded yet, history is fetched until it is - a
    // bounded number of pages, after which the room stays at its newest
    // message like before.
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
            // What "at the end" means is only known once the view has settled.
            // Two unread messages fit on the screen, so the jump lands at the
            // end and the room is being read; a longer run does not, and it is
            // not. Without asking, the room stayed parked: new messages no
            // longer followed, and nothing counted as read either.
            followCheck.restart()
            return
        }
        // Nothing has arrived yet. The core answers `timeline.open` before the
        // rows are there - they come afterwards through the diff stream - and
        // this runs a millisecond later, so an empty model here means "not yet",
        // not "not in it". Asking for older history at that moment searches a
        // timeline that has not had its first batch, and spends a page of the
        // budget doing it. Waiting is bounded, because a room that never
        // delivers a row must not hold the view forever.
        if (matrix.timeline.count === 0 && page.unreadEmptyRounds > 0) {
            page.unreadEmptyRounds--
            unreadRetry.restart()
            return
        }
        if (page.unreadPagesLeft > 0 && !matrix.timelineAtStart) {
            // One request at a time; the timer brings us back either way. On a
            // timer and not on the row count, for the same reason the permalink
            // jump is: a page of history can arrive full of events that render
            // as nothing, and then the count never changes.
            if (!matrix.paginating) {
                page.unreadPagesLeft--
                matrix.loadOlder()
            }
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
        // Verifying somebody is exactly the moment the warning should stop
        // naming them. Without this the throttle keeps the old answer for up to
        // a minute, and a warning that returns after the user did what it asked
        // is the warning they switch off.
        onVerificationChanged: page.refreshRecipients(true)

        onRecipientsChecked: {
            if (roomId === page.roomId) {
                page.unverifiedUsers = users
            }
        }
    }

    // Deliberately not closed on leaving: keeping the subscription alive makes
    // stepping back into the room instant instead of rebuilding the timeline
    // every time. The core keeps exactly one, and opening another room
    // replaces it.

    onStatusChanged: {
        // Which room is on screen — the room whose notifications are pointless
        // because the user is looking at it. Not the same as the open room:
        // the core keeps that room's timeline subscribed after the page is
        // gone, deliberately, so that one names the room last visited.
        if (status === PageStatus.Active) {
            matrix.setVisibleRoom(roomId)
        } else if (status === PageStatus.Deactivating
                   || status === PageStatus.Inactive) {
            matrix.setVisibleRoom("")
            page.markReadIfDue()
        }

        if (status === PageStatus.Active && !invited) {
            // Returning from the pinned view the shared timeline must show
            // live events again. For a timeline that is already live this is
            // a no-op.
            matrix.openRoom(roomId)
            // Not marked read outright any more: the view may open at the
            // first unread message, and everything below it is unread until
            // the user gets there.
            readTimer.restart()
            tryJump()
            // The room's own state, again: encryption can have been switched
            // on one page further in while this one was covered, and the lock
            // at the top would otherwise still stand open.
            matrix.loadRoomInfo(roomId)
            // Devices may have been verified (or new ones appeared) while the
            // page was covered; re-check so the warning stays current.
            refreshRecipients()

            // Extra work goes inside this handler: QML refuses a type that
            // binds onStatusChanged twice, and the page then fails to load
            // with nothing but "could not load page" to go on.
            if (pendingAction) {
                var action = pendingAction
                pendingAction = null
                if (action.kind === "reply") {
                    beginReply(action.eventId, action.senderName, action.body)
                } else if (action.kind === "edit") {
                    beginEdit(action.eventId, action.body)
                } else if (action.kind === "sendMedia") {
                    submitMedia(action)
                } else if (action.kind === "delete") {
                    // The countdown belongs on the page that stays. Started on
                    // the actions page it would be executed by Silica the
                    // moment that page pops, which is the opposite of a way
                    // back.
                    confirmDelete(action.eventId, action.txnId, action.unsent)
                }
            }
        }
    }

    // Leaving is irreversible and used to be one accidental tug away, so it
    // takes two deliberate steps: a dialog that names the room, then the
    // remorse as the undo.
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
                        // Silica executes a running remorse the moment its
                        // page deactivates ("if the page is changed then
                        // execute immediately", RemorsePopup.qml), so going
                        // back used to *complete* the countdown instead of
                        // stopping it — the reported way to lose a room by
                        // accident. Closing the app is the same story from the
                        // other side: the page is destroyed without ever
                        // reaching Deactivating, and minimising leaves the page
                        // untouched while the countdown runs out unseen. Only a
                        // countdown that ran out on its own page, with the app
                        // in front of the user, counts.
                        if (page.status !== PageStatus.Active || !Qt.application.active) {
                            return
                        }
                        matrix.leaveRoom(page.roomId)
                        if (pageStack.currentPage === page) {
                            pageStack.pop()
                        }
                    })
    }

    // Deleting is not undoable and it used to be one tap away, at the bottom of
    // a menu that opens on a long press - the same reach that leaving a room
    // has, and that one has had its countdown since the report. The message
    // goes when the countdown runs out, on this page, with the app in front of
    // the user; anything else cancels it. A send that failed is discarded
    // rather than deleted: it never reached anybody, and it has no event id.
    function confirmDelete(eventId, txnId, unsent) {
        page.activeRemorse = Remorse.popupAction(
                    page,
                    unsent ? qsTr("Discarding") : qsTr("Deleting"),
                    function() {
                        // The same test the room's own remorse makes: Silica
                        // executes a running countdown the moment its page
                        // deactivates, and minimising leaves it running unseen.
                        if (page.status !== PageStatus.Active || !Qt.application.active) {
                            return
                        }
                        matrix.deleteMessage(eventId || "", txnId || "")
                    })
    }

    // What the row says instead of "not decryptable". The SDK works the reason
    // out and the core forwards it as a key; without the sentence every such
    // message reads the same, and the difference between "the sender withheld
    // the key from this device" and "older than this device" is exactly what
    // decides whether anything can be done about it.
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
        // Only when this page is the one on screen. The pinned overview calls
        // this on the page underneath, and at that moment the shared model
        // still holds the *pinned* view: the row is found there, the view is
        // placed on it, and the switch back to the live timeline throws that
        // away a fraction of a second later. That is exactly what was
        // reported - the right message for a blink, then the end of the room.
        // Left standing, the jump runs when the live timeline is back.
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
        // Nothing to look in yet: coming back from the pinned view empties the
        // model and asks the core for the live one. Looking anyway would miss
        // and start paging through a history that is about to arrive by
        // itself. Bounded, so a room that never delivers stops asking.
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
            // Retried on a timer and not only on the row count: a page of
            // history can arrive full of events that render as nothing, and
            // then the count never changes and the jump dies without a word.
            // That is exactly why this never worked — "no new rows" and "no
            // more history" look the same from up here.
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

    // A timeline that does not fill the screen cannot be scrolled, so reaching
    // the top — the only trigger for loading older messages — never fires. A
    // freshly joined room arrives with nothing but its own join event and would
    // sit there forever showing two rows. Keep asking for history until there
    // is something to scroll or the room's beginning is reached.
    //
    // Bounded, because "the list did not grow" and "there is no more history"
    // are two different things. A page can come back full of events that render
    // as nothing — a stretch of membership changes is enough — and an unbounded
    // fill then asks again for as long as the room has history. Against a
    // homeserver that rate-limits (matrix.org does) each of those requests is
    // retried by the SDK for minutes, so the app ends up waiting on a queue it
    // produced itself, and the room looks frozen. After a few fruitless rounds
    // the automatic fill stops and leaves it to the user, who still has the
    // pull-down entry and the pull-to-top.
    property int fillEmptyRounds: 0
    readonly property int fillEmptyLimit: 3
    /// How tall the conversation was when the last automatic round started,
    /// -1 for none yet.
    ///
    /// Height, not row count. The loop runs while the content is shorter than
    /// the screen, so height is the thing it is trying to change - and there
    /// are rows that raise the count without raising it by a pixel: a pure
    /// profile change renders as nothing at all. A room with a stretch of those
    /// therefore never stopped asking, because every round "grew" and reset the
    /// counter. Measured on the device: fifty-six rows to two hundred and
    /// twenty-six in four seconds. This is the same lesson as the original
    /// unbounded fill, applied to the size that decides rather than the one
    /// that was easiest to read.
    property real fillLastHeight: -1

    function fillScreen() {
        if (page.invited || !matrix.timelineReady || matrix.paginating
                || matrix.timelineAtStart || timelineView.count === 0) {
            return
        }
        if (timelineView.contentHeight > timelineView.height) {
            return
        }

        // Whether the last round brought anything is decided here and not when
        // its reply arrived. The rows reach the model through the diff stream,
        // a path separate from the reply, and they regularly arrive after it —
        // measured on the device: "0 new rows" and thirty rows in the same
        // second. By the time the fill is asked again, they are applied.
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

        // The round is over, so the next one may start — and if this one
        // brought nothing at all, no diff will arrive to trigger the fill, so
        // this is the only thing that keeps a short room moving. This used to
        // hang off busyChanged, which fires for every command in the whole
        // application: a downloading avatar then drove the history fetching.
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

    // The switch into the replacement room is over — the application window
    // opens it. A failed join has to clear the banner's wait just as well, or
    // it keeps announcing a move that is no longer happening.
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

    // The room's menu has to be reachable from anywhere in the conversation,
    // so it does not hang on the timeline — whose top is the oldest loaded
    // message, arbitrarily far away — but on a page-sized flickable that
    // cannot scroll. Dragging the fixed strip below opens the menu; dragging
    // the conversation scrolls it, because the list is a flickable of its own.
    // This is how other chat clients on this platform solve the same problem.
    SilicaFlickable {
        id: roomView

        anchors.fill: parent
        contentHeight: height
        contentWidth: width
        // Nothing scrolls here except the pulley's own overdrag. A stray
        // positive contentY would shift the whole page up and leave it there,
        // which is why other clients clamp it too.
        onContentYChanged: {
            if (contentY > 0) {
                contentY = 0
            }
        }

        // Conditions belong on the entries, never on the menu: hiding the
        // pull-down because one entry does not apply takes every other entry
        // with it, and the page ends up with no way out.
        PullDownMenu {

            // Order matters here, and it is the reverse of reading order: the
            // *last* entry declared is the bottom one, the one a short tug
            // lands on. Leaving used to sit there and was picked by accident;
            // it is now the topmost entry, so reaching it means pulling the
            // menu all the way open. The harmless everyday entry has the
            // short tug instead.
            MenuItem {
                // Deliberately without a visibility condition: an unwanted
                // invitation has to be refusable and a joined room has to be
                // leavable, so this entry is the way out in either state.
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

            // The banner is the obvious way, but a menu entry is the one a user
            // looks for — and the banner sits above a conversation that is
            // scrolled to its end, where nothing draws the eye upwards.
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

            // Everything about the room rather than about the running
            // conversation lives one page further in: members, pinned
            // messages, inviting, encryption, the room's own details. This
            // menu had grown to ten entries, which is barely draggable on a
            // small screen in landscape.
            MenuItem {
                text: qsTr("Room info")
                onClicked: pageStack.push(Qt.resolvedUrl("RoomInfoPage.qml"), {
                                              roomId: page.roomId,
                                              roomName: page.roomName,
                                              invited: page.invited
                                          })
            }

            // Last, so the shortest tug reaches it: reading further back is
            // what one reaches for while in the conversation, and the entries
            // above are all about leaving it.
            MenuItem {
                text: qsTr("Load older messages")
                visible: !page.invited && !matrix.timelineAtStart
                // Only a running pagination disables this. It used to be the
                // global busy flag, which is true while *any* command waits for
                // an answer — the entry then sat grey because some unrelated
                // thumbnail was still downloading.
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
                // Out from under the camera cutout. This strip is the room's
                // own, not a `PageHeader` - which is exactly why it needed
                // saying: Silica gives its own header the same margin by the
                // same rule (`_minimumTopMargin` in PageHeader.qml), so every
                // page that uses one is already clear and this one was not.
                // Measured on a device with a wide cutout: the notch edge cut
                // through the room name at the height of a lower-case letter.
                //
                // Portrait only, as Silica has it: in landscape the cutout is
                // at the side and a top margin would be a gap for nothing. On
                // a device without one the rectangle is empty and this is zero,
                // so it costs the other phones nothing.
                topMargin: page.orientation === Orientation.Portrait
                           ? Screen.topCutout.height : 0
            }
            height: headerColumn.height + 2 * Theme.paddingMedium
            // The room's name leads to what the room is — including, for an
            // invitation, the topic that decides whether to accept it. The
            // member list sits one level further in.
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

                // The lock leads the room's name, a gap away from it. It says
                // what the second line used to spell out in words: closed and
                // green for an encrypted room, open and red for one without.
                // A shape at the top of the room is looked at; a sentence
                // under the name was read once and never again.
                Row {
                    anchors.right: parent.right
                    spacing: Theme.paddingLarge

                    LockIcon {
                        id: headerLock

                        // Against the Row, not against the name beside it: a
                        // positioner sets its children's y, and two of them
                        // reaching into each other for it is a fight nobody
                        // needs.
                        anchors.verticalCenter: parent.verticalCenter
                        // An invitation says nothing reliable about the room's
                        // encryption yet, and neither does a room that has not
                        // answered: both claim nothing rather than the wrong
                        // thing.
                        visible: !page.invited && page.encryptionKnown
                        size: Math.round(Theme.fontSizeLarge * 1.2)
                        locked: page.encrypted
                        // Faint, both states alike: the name is what the strip
                        // is for, and the lock is the second thing the eye
                        // lands on, not the first. Set by eye on the device -
                        // half was still too present.
                        opacity: 0.30
                    }

                    Label {
                        id: nameLabel

                        // The name takes what the lock leaves. Reading the
                        // column's width is safe: it comes from the strip's
                        // anchors, never from anything inside this Row.
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
                    // Only what the lock cannot say. Being cut off from the
                    // server outranks anything about the room itself: a
                    // silently stale conversation is the one thing the user
                    // cannot see for themselves. With nothing to report the
                    // line is gone entirely, and the strip is one line high.
                    visible: text.length > 0
                    text: matrix.syncState === "offline"
                          ? qsTr("Offline — waiting for the network")
                          : page.invited ? qsTr("Invitation") : ""
                }
            }
        }

        // An upgraded room takes no new messages: the upgrade raises the power
        // level for sending, so the conversation just stops while the room goes
        // on looking alive. This says so and leads to the room that replaced it
        // — above the pinned banner, because everything else about this room is
        // history now.
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
                    // The reason is free text from whoever upgraded the room and
                    // may well say nothing useful, so the way out comes first
                    // and the reason only fills what is left of the line.
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

        // Pinned messages sit directly under the room's name while the
        // conversation scrolls underneath — the banner lives outside the list,
        // so it never moves.
        BackgroundItem {
            id: pinnedBanner

            anchors {
                left: parent.left
                right: parent.right
                top: tombstoneBanner.bottom
            }
            height: visible ? Theme.itemSizeExtraSmall : 0
            visible: !page.invited && matrix.pinnedEventIds.length > 0
            // The banner is one line high and stands outside the list: a preview
            // that grew taller would paint over the whole conversation instead of
            // being cut off.
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
                // The newest pin's text, the way Telegram shows it; the count is
                // the fallback while the text is still loading (or for a pin
                // whose body cannot be read), and a prefix when there are more.
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

            // Air under the last message, and not for looks. The read mark -
            // the eye and its number - sits on the bottom line of a bubble, and
            // the area that takes the tap reaches a finger's width below that,
            // which is exactly where the message field begins. A last message
            // sitting flush on the field therefore had a mark that could not be
            // hit, and the names it unfolds appeared behind the field. One line
            // of the mark's own height plus that reach, so the bottom of the
            // conversation always stands clear of the composer.
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

                // Without this the menu's "load older messages" simply does
                // nothing once the history is complete, which reads like a
                // broken button.
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
                // Where reading stopped. The SDK only produces this row while
                // receipt tracking is on, so it appears with the setting.
                readonly property bool isMarker: model.kind === "marker"
                // Everything read puts the marker at the very end, where a line
                // under the last message says nothing. It belongs between what
                // was read and what was not, or nowhere.
                // Not while the conversation is being followed live. Every
                // arriving message is unread for the moment before the receipt
                // goes out, so the marker slips in above it and the line
                // flickered under every single message in an open room. It is
                // there to say "this is where you stopped", which only means
                // something to someone who is not at the end anyway.
                readonly property bool showMarker: isMarker
                                                   && index < timelineView.count - 1
                                                   && !page.followTail
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
                // Only a body that visibly carries a web link pays the rich-text
                // path; everything else stays plain text. Behind a setting,
                // default off: a tappable link is attack surface.
                // Never for a row that carries a file. What such a row shows is
                // the sender's caption, and the caption is passed on as it
                // arrived - putting it in front of a markup parser would hand
                // a stranger `<img src="http://…">`, which StyledText fetches.
                readonly property bool hasLink: model.kind === "message"
                                                && !model.media
                                                && ((settings.clickableLinks
                                                     && /https?:\/\//.test(model.body || ""))
                                                    || MatrixLinks.hasAddress(model.body))
                // The message carried an HTML body and the core made markup of
                // it. Everything in that string was written by the core itself
                // — the sender's characters are all escaped — so it can go to
                // StyledText the same way a linkified body does.
                readonly property bool hasFormatted: model.kind === "message"
                                                     && !model.media
                                                     && (model.formatted || "").length > 0

                // The body as markup, or empty when plain text will do. One
                // property for both the format and the text, so the two can
                // never disagree - a Label parsing markup that was not built
                // for it is how a sender's characters become tags.
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

                // Calls and membership changes are not messages, but a room made
                // only of them must not look empty: they show as a centred line.
                // Pure profile changes (name, avatar) are noise — Matrix writes
                // one into every joined room — so their rows collapse. They stay
                // in the model: dropping them would desynchronise the indices
                // from the SDK's diffs.
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

                // A video is shown by the preview its sender supplied; without one
                // it stays a plain attachment line.
                // What a sender declares about a picture. `sourceSize` is a
                // hint the decoder may ignore - an interlaced PNG or a GIF is
                // decoded whole - so a picture whose own header claims absurd
                // dimensions is not loaded at all.
                // Dimensions the decoder can be trusted with. Separate from
                // the size test below, because the two have different answers:
                // a picture whose size nobody declared may still be opened on a
                // tap, one whose declared dimensions are absurd may not - Qt's
                // `sourceSize` is a hint an interlaced PNG is free to ignore,
                // and the decoder then takes width × height × 4 bytes.
                readonly property bool saneDimensions: !model.media
                        || ((model.media.width || 0) <= 8192
                            && (model.media.height || 0) <= 8192)

                readonly property bool sanePicture: !model.media
                        || (saneDimensions
                            // The declared size, before anything is fetched:
                            // the ceiling in the core only sees bytes that are
                            // already in memory.
                            //
                            // Nothing is required of that figure. 0.25.0 asked
                            // for it to be there and dropped the preview where
                            // it was not - which took every picture this app
                            // sends itself, because the SDK writes an empty
                            // `info` when it is handed none (fixed in the core
                            // as well). A missing size is the common case, not
                            // a suspicious one; what protects the decoder is
                            // the second ceiling in `media::fetch`, which
                            // weighs the bytes that actually arrived and does
                            // not care what was declared.
                            && (model.media.size || 0) <= 100 * 1024 * 1024)

                readonly property bool hasPreview: sanePicture
                                                   && (isImage
                                                       || (isVideo && !!model.media.thumbnailSource))

                width: timelineView.width
                contentHeight: isBubble
                               ? Math.max(bubble.height,
                                          row.isOwn ? 0 : page.avatarSize)
                                 + Theme.paddingMedium
                               : (model.kind === "date"
                                  ? dayLabel.height + Theme.paddingLarge
                                  : (isSystem
                                     ? systemLabel.height + Theme.paddingLarge
                                     : (showMarker ? Theme.paddingLarge : 0)))

                // Only real messages react; dividers are not something to press.
                enabled: model.kind === "message"
                _showPress: false

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

                    // The entries below carry `!page.isLandscape` because in
                    // landscape the screen is 1080 px high and the
                    // conversation keeps under 600 of them — four menu rows,
                    // where a message's own menu needs six. A context menu
                    // cannot scroll (Silica's is a plain column, and flicking
                    // closes it), so the last entries were simply out of
                    // reach. There they move to a page instead, reached
                    // through "More…" below.
                    MenuItem {
                        // Starting one, not only answering in one that exists:
                        // the marker on a message opens a thread that is
                        // already there, and until now nothing opened one.
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
                        // Only where this account may write the room's pinned
                        // list. `!== false` and not a plain test: until the
                        // room's answer is in, the map has no entry, and a
                        // slow answer must not take an action away from
                        // somebody who has it.
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
                        // Also the only way out for a message whose send
                        // failed for good: it has no event id, and without
                        // this entry it would sit in the room for ever.
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
                                                      // Copied out, not handed
                                                      // over: the model row is
                                                      // gone once it leaves the
                                                      // cache.
                                                      item: {
                                                          "id": model.id,
                                                          "msgtype": model.msgtype,
                                                          "media": model.media
                                                      }
                                                  })
                    }
                }

                // The sender's picture, left of their bubble. Only for other
                // people: on one's own messages it would say nothing, and the
                // right-hand side is where they sit.
                //
                // The address is the same for every message of a sender, and
                // Avatar keys its request by exactly that — so a conversation
                // of two hundred messages from five people downloads five
                // pictures, not two hundred.
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

                    // Tap opens the sender's profile. The margin stays inside
                    // the gap to the bubble (paddingSmall), or it would take
                    // taps meant for the message; the long press still has to
                    // reach the message's own menu.
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
                        // A constant, never the avatar's own width: reading a
                        // sibling's width here and its height back there is how
                        // this delegate earns a binding loop.
                        leftMargin: Theme.horizontalPageMargin
                                    + page.avatarSize + Theme.paddingSmall
                        top: parent.top
                    }
                    // Width follows the column, which sizes itself from the texts'
                    // natural widths. Deriving it from the labels' own width would
                    // close the loop, since their width comes from the bubble.
                    width: bubbleColumn.width + 2 * Theme.paddingMedium
                    height: bubbleColumn.height + 2 * Theme.paddingMedium
                    radius: Theme.paddingMedium
                    // Both fills stay faint by default: the theme's text
                    // palette is tuned to the page background, and an opaque
                    // secondaryHighlightColor fill can land on any lightness
                    // depending on the ambience — one tester's own bubble was
                    // light under a light palette and swallowed every colour
                    // on it. A tint keeps the palette readable everywhere;
                    // the stronger one marks the own side. Colour and opacity
                    // can be overridden per side on the appearance page.
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

                        // The widest a bubble's text may get before wrapping.
                        // Someone else's bubble starts further right, so it
                        // gets correspondingly less.
                        property real maxTextWidth: timelineView.width * 0.82
                                                    - 2 * Theme.paddingMedium
                                                    - (row.isOwn ? 0 : page.avatarSize
                                                                      + Theme.paddingSmall)

                        anchors {
                            left: parent.left
                            top: parent.top
                            margins: Theme.paddingMedium
                        }
                        // No explicit width: a Column is as wide as its widest
                        // child. Every child below derives its width from its own
                        // implicit width, never from this column, which is what
                        // keeps the layout free of binding loops.
                        spacing: Theme.paddingSmall / 2

                        // Which edge the children stand on. An own bubble is
                        // anchored right and grows to the left, so everything in
                        // it has to hang off the right edge - otherwise the read
                        // marks unfolding under a three-letter answer widen the
                        // bubble and drag the text and the reactions leftwards
                        // with it. Somebody else's bubble grows to the right,
                        // where left is the edge that stays put.
                        //
                        // This is x from the parent's width, not width from it:
                        // the column still takes its width from the children,
                        // and nothing here reads its own position back.
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

                        // What this message answers, as a compact quote. The
                        // bar stands to the LEFT of the quoted lines, the way
                        // the other messengers draw it, so name and text
                        // visibly hang off it — a bar across the top made the
                        // quoted sender and the author's name above look
                        // interchangeable in a received bubble.
                        // Wrapped so the quote can be tapped: a MouseArea cannot be a child
                        // of the Row itself (Row positions its children and forbids their
                        // anchors), and the Item takes its size from the Row — parent reads
                        // child, child reads parent, never both directions in one subtree.
                        Item {
                            id: replyBlock

                            // The anchor belongs here, on the child of the
                            // column - not on the Row inside, whose parent is
                            // this Item and exactly as wide as itself, where it
                            // was a no-op. Without it the quote was the one
                            // thing in an own bubble that stayed on the left
                            // when the bubble grew.
                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                            // Not for a row that belongs to a thread. The SDK
                            // marks every threaded event as a reply to the one
                            // before it in its thread, for clients that cannot
                            // do threads, and only suppresses that inside a
                            // thread-focused timeline. In the room it drew a
                            // quote of the previous thread message under every
                            // thread reply - a quote no other client shows, and
                            // one fetch each to fill it in.
                            // A real reply inside a thread loses its quote in
                            // the room too - the SDK does not say which of the
                            // two kinds this is, so the coarse rule is the only
                            // one available. The thread view still shows it.
                            visible: !!model.replyTo
                                     && (model.threadRoot || "").length === 0
                            width: replyQuote.width
                            height: replyQuote.height

                            Row {
                                id: replyQuote

                                // No explicit width: like every sibling, the row
                                // is as wide as its content and each child sizes
                                // bottom-up. An earlier form derived a width from
                                // the children's implicit widths while the
                                // children read it back — the mixed directions
                                // the width rule forbids, and the device journal
                                // duly reported the loop.
                                spacing: Theme.paddingSmall

                                Rectangle {
                                    width: Theme.paddingSmall / 2
                                    // The texts' height, not the parent's —
                                    // reading the parent back would close the
                                    // loop again.
                                    height: quoteTexts.height
                                    radius: width / 2
                                    color: Theme.rgba(Theme.highlightColor, 0.6)
                                }

                                // A quoted picture as a picture. Its body is
                                // the file name, which says nothing about what
                                // was answered - one had to open the original
                                // to know.
                                Image {
                                    id: quoteThumb

                                    readonly property var quotedMedia: model.replyTo
                                                                       ? model.replyTo.media : null
                                    readonly property string mediaKey: model.replyTo
                                                                       ? model.replyTo.eventId + "/quote" : ""

                                    visible: !!quotedMedia
                                             && (model.replyTo.msgtype === "m.image"
                                                 || model.replyTo.msgtype === "m.video")
                                    width: visible ? Theme.itemSizeSmall : 0
                                    height: width
                                    fillMode: Image.PreserveAspectCrop
                                    clip: true
                                    asynchronous: true
                                    // A fixed decode size: the quote is a
                                    // thumbnail, and a picture from outside
                                    // must not be able to ask for arbitrary
                                    // memory.
                                    sourceSize.width: Theme.itemSizeSmall * 2
                                    sourceSize.height: Theme.itemSizeSmall * 2

                                    // Also on a change of key, not only once:
                                    // the delegate is recycled while scrolling
                                    // and would otherwise keep the picture of
                                    // the row it was last used for.
                                    Component.onCompleted: load()
                                    onMediaKeyChanged: {
                                        source = ""
                                        load()
                                    }

                                    function load() {
                                        if (!visible || mediaKey.length === 0) {
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

                                Column {
                                    id: quoteTexts

                                    // What the labels may use once the bar, the
                                    // picture and the spacing took their share.
                                    readonly property real maxWidth: bubbleColumn.maxTextWidth
                                                                     - Theme.paddingSmall * 1.5
                                                                     - (quoteThumb.visible
                                                                        ? quoteThumb.width + Theme.paddingSmall
                                                                        : 0)

                                    Label {
                                        id: quoteSender

                                        width: Math.min(implicitWidth, quoteTexts.maxWidth)
                                        // An empty line above the quote reads as
                                        // a rendering glitch; the box collapses
                                        // to the body line until the sender is
                                        // known.
                                        visible: text.length > 0
                                        font.pixelSize: Theme.fontSizeExtraSmall
                                        color: appearance.nameColor.length > 0
                                               ? appearance.nameColor : Theme.highlightColor
                                        truncationMode: TruncationMode.Fade
                                        textFormat: Text.PlainText
                                        // While the quoted event is still
                                        // being fetched its sender is not
                                        // known - the same ellipsis the body
                                        // uses says so, rather than a line
                                        // that silently collapses.
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

                                        // Fixed to the full text width, never to
                                        // the own implicit width: with wrap and
                                        // elide the implicit width follows the
                                        // set width in this Qt, and reading it
                                        // back is a binding loop (the journal
                                        // caught the churn). A reply bubble is
                                        // therefore always full width — which is
                                        // also how the quote reads best — and
                                        // the box cannot collapse while the
                                        // quoted message is still being fetched.
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
                                            if (quoteThumb.visible && model.replyTo.media
                                                    && body === model.replyTo.media.filename) {
                                                return ""
                                            }
                                            return body.length > 0 ? body : "…"
                                        }
                                        visible: text.length > 0
                                    }
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

                        // Attachments are fetched on demand and cached on disk, so
                        // scrolling past the same image does not download it twice.
                        Image {
                            id: attachment

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            visible: row.hasPreview
                            width: row.hasPreview ? bubbleColumn.maxTextWidth : 0
                            height: visible
                                    ? (model.media && model.media.width > 0 && model.media.height > 0
                                       ? width * model.media.height / model.media.width
                                       : width * 0.75)
                                    : 0
                            fillMode: Image.PreserveAspectFit
                            asynchronous: true
                            // A ceiling on what is decoded, not only on what is
                            // drawn: without it the sender decides the
                            // allocation, and a 30000-pixel picture is a few
                            // hundred kilobytes on the wire and gigabytes in
                            // memory. The height follows the aspect ratio.
                            // Both axes. With only a width the decode follows
                            // the aspect ratio, so a 64x200000 picture - a few
                            // kilobytes on the wire - still decodes to
                            // gigabytes. The bound has to be an area, not an
                            // edge.
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
                                // A sender-provided thumbnail is already small and,
                                // in encrypted rooms, the only one that exists. A
                                // video has no other preview at all.
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
                                // Not while waiting for something that is not
                                // coming: a refused or failed download used to
                                // leave this turning for the life of the page.
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

                            // Measured by the invisible twin below, never by
                            // the own implicit width: with wrap the implicit
                            // width follows the laid-out width in this Qt,
                            // and reading it back is the width binding loop
                            // the device journal kept reporting. The twin
                            // never wraps, so its implicit width is the
                            // text's natural width and depends on nothing
                            // but text and font.
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
                            // Plain text on purpose: a message body is untrusted,
                            // and AutoText would render anything that looks like
                            // HTML — including <img> that phones home. Only a
                            // body with a detected web link switches to
                            // StyledText, and then every character has been
                            // escaped by linkifyBody first.
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
                                // Whatever carries a picture or a film and did
                                // not get a preview. A video never gets one
                                // without a thumbnail; a picture loses it when
                                // the sender declared no size, and then this
                                // line is the only way in - it used to be
                                // nothing but text, and the attachment could
                                // only be reached through "save" in the menu.
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

                        // What the SDK will not vouch for. Red is a message
                        // that is not what it claims to be; grey is one it
                        // cannot check. Both are silent otherwise.
                        Label {
                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                            width: Math.min(implicitWidth, bubbleColumn.maxTextWidth)
                            visible: row.isBubble && !!model.shield
                            font.pixelSize: Theme.fontSizeExtraSmall
                            color: model.shield && model.shield.level === "red"
                                   ? Theme.errorColor : Theme.secondaryColor
                            textFormat: Text.PlainText
                            text: page.shieldText(model.shield)
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

                        // Reactions, and only where there are any: a delegate
                        // is rebuilt on every scroll, so the messages without
                        // one pay a single inactive Loader instead of a row of
                        // items they would never fill.
                        //
                        // A Row, not a Flow: a Flow's width has to come from
                        // its own implicit width, and reading that back is the
                        // binding loop this project has paid for three times -
                        // it showed up as chips drawn across the message text.
                        // Everything here reads its children, never its parent.
                        Loader {
                            id: reactionBar

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            active: row.hasReactions
                            visible: active
                            // No width or height here on purpose. A Loader
                            // that has been given a size resizes the item it
                            // loaded to that size - so taking the size *from*
                            // that item is a circle, and Qt answers a circle
                            // with zero. The row then painted its chips at the
                            // bubble's corner, across the sender name and the
                            // text. Left undefined, the Loader takes the size
                            // of what it loaded, which is the whole point.
                            sourceComponent: Row {
                                // No padding properties: they arrived in
                                // QtQuick 2.7, and 5.6 is this project's
                                // ceiling. The column's own spacing separates
                                // the bar from the text.
                                spacing: Theme.paddingSmall

                                Repeater {
                                    model: row.reactionsShown

                                    BackgroundItem {
                                        width: chip.width + 2 * Theme.paddingSmall
                                        height: chip.height + Theme.paddingSmall
                                        // The delegate's own model is out of
                                        // scope inside the Loader; the row
                                        // hands the event id over instead.
                                        onClicked: {
                                            // Drawn is filtered and cut to 32
                                            // characters, sent is the raw key.
                                            // Where the two differ, the chip
                                            // shows less than it would publish -
                                            // a stranger can hide text behind
                                            // zero-width characters and have it
                                            // sent under this account's name,
                                            // signed, from a single tap. Taking
                                            // back one's own stays possible;
                                            // joining a reaction one has not
                                            // seen in full does not.
                                            var sendKey = modelData.sendKey || modelData.key
                                            if (!modelData.mine && sendKey !== modelData.key) {
                                                page.showNotice(qsTr("This reaction hides text and was not sent"))
                                                return
                                            }
                                            matrix.toggleReaction(row.rowEventId, sendKey)
                                        }
                                        // Held down, the chip says who set it -
                                        // under the message, where the readers
                                        // stand too. It takes the long press
                                        // away from the message's own menu for
                                        // the width of a chip; everywhere else
                                        // on the bubble that menu is unchanged.
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

                                            // One rule for drawing an emoji,
                                            // in one place (EmojiItem): a
                                            // picture the user brought,
                                            // otherwise the character.
                                            EmojiItem {
                                                anchors.verticalCenter: parent.verticalCenter
                                                character: modelData.key
                                                // Half again as large as the
                                                // icon size it was drawn at:
                                                // reported from a small screen,
                                                // where an emoji at that size
                                                // is a coloured speck and the
                                                // difference between two of
                                                // them cannot be made out.
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

                                // A room can carry more distinct reactions than
                                // a bubble has room for; the rest are counted
                                // rather than pushing the bubble off the screen.
                                Label {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: row.reactionsHidden > 0
                                    font.pixelSize: Theme.fontSizeExtraSmall
                                    color: Theme.secondaryColor
                                    text: "+" + row.reactionsHidden
                                }
                            }
                        }

                        // The time, and beside it how many have read this far.
                        // An Item around the line, because the read mark is a
                        // drawn icon and an icon cannot sit inside a string.
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
                                            // The names below can be the widest
                                            // thing in the bubble; without them
                                            // in this list the time would stand
                                            // right of nothing.
                                            readersLabel.visible ? readersLabel.width : 0,
                                            reactionBar.visible ? reactionBar.width : 0,
                                            metaContent.width)
                            height: metaContent.height

                            Row {
                                id: metaContent

                                anchors.right: parent.right
                                spacing: Theme.paddingSmall

                                Label {
                                    id: metaLabel

                                    anchors.verticalCenter: parent.verticalCenter
                                    font.pixelSize: Theme.fontSizeTiny
                                    // A message that did not get out is the one
                                    // thing in this line worth a colour: half
                                    // opacity alone reads as "still going" and
                                    // let someone send the same text a second
                                    // time.
                                    color: model.sendState === "failed"
                                           ? Theme.errorColor : Theme.secondaryColor
                                    // Only messages carry a timestamp; the shared
                                    // delegate instantiates this label for every
                                    // row regardless.
                                    text: row.isBubble
                                          ? (matrix.pinnedEventIds.indexOf(model.eventId) >= 0 ? "📌 " : "")
                                            + (model.sendState === "failed"
                                               ? qsTr("not sent") + " · " : "")
                                            + (model.edited === true ? qsTr("edited") + " · " : "")
                                            + Format.formatDate(new Date(model.timestamp), Formatter.TimeValue)
                                          : ""
                                }

                                // On the newest own message the others have read
                                // up to, which is where a receipt actually means
                                // something - it usually hangs on their own
                                // latest message, not on ours. Gone entirely
                                // while the status is switched off: nothing is
                                // tracked then.
                                //
                                // An eye and a number instead of the words: the
                                // line already carries the time, and "read by
                                // three" took more of the bubble than the fact
                                // is worth.
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
                                            // A step larger than the digit next
                                            // to it, the way the icon in the
                                            // composer stands over its text.
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

                                    // Both the eye and the number, and a good
                                    // deal of air around them: the mark is a
                                    // couple of millimetres tall and a finger
                                    // is not. Negative margins grow what can be
                                    // hit without moving anything.
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

                        // Who they are, as one line rather than a list: a name
                        // per row would push the conversation off the screen
                        // for an aside.
                        Label {
                            id: readersLabel

                            anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                            visible: page.namesEventId === model.eventId
                                     && page.namesText.length > 0
                            // Sixteen names do not belong in a column three
                            // characters wide. The names widen the bubble to
                            // the same limit every other text in it obeys, and
                            // only what is longer than that wraps - an answer
                            // of "Yes" used to hand them a bubble the width of
                            // those three letters and let them run down it.
                            //
                            // Measured by the twin below, never by the own
                            // implicit width: with wrap that follows the
                            // laid-out width in this Qt, and reading it back is
                            // the binding loop this delegate has paid for
                            // already. Same rule as the message body.
                            width: Math.min(readersMeasure.implicitWidth,
                                            bubbleColumn.maxTextWidth)
                            // Left, unlike the time line above it: this runs
                            // over several lines, and a wrapped block with a
                            // ragged left edge is read line by line instead of
                            // at a glance. The message body in the same bubble
                            // sits left too.
                            horizontalAlignment: Text.AlignLeft
                            wrapMode: Text.Wrap
                            font.pixelSize: Theme.fontSizeTiny
                            color: Theme.secondaryColor
                            textFormat: Text.PlainText
                            // Only the row the names belong to carries them.
                            // The string is the page's, so without this test
                            // every row in the timeline would hold it and its
                            // hidden twin would lay it out.
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
                        verticalCenter: parent.verticalCenter
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
                    // Read where it is actually read: the room used to be
                    // marked read only on entering, so a message arriving
                    // while it was open stayed unread in the chat list until
                    // the room was entered a second time.
                    readTimer.restart()
                }
            }

            onMovementEnded: {
                page.followTail = atYEnd
                // A hand on the list outranks the search for the read marker:
                // being carried off to a jump one has just scrolled away from
                // is worse than not being taken there at all.
                page.unreadFromId = ""
                unreadRetry.stop()
                // Scrolling down to the newest message is the other moment
                // something becomes read.
                if (atYEnd) {
                    readTimer.restart()
                }
            }

            // Reaching the top is the natural moment to fetch older messages.
            // Asked for by the user, so the fill's give-up counter does not
            // apply here — only a pagination already running holds it back, and
            // a room whose beginning is already loaded, where there is nothing
            // left to ask for. Without that second test every touch of the top
            // edge sent another request into the void; harmless against a fast
            // homeserver, another rate-limited round trip against a busy one.
            onAtYBeginningChanged: {
                if (atYBeginning && count > 0 && !matrix.paginating
                        && !matrix.timelineAtStart) {
                    matrix.loadOlder()
                }
            }

            // ...but a list too short to scroll never reaches it. The count side of
            // this sits in onCountChanged above — a second handler for the same
            // signal is not an addition, it is a load error that kills the page.
            // Two things hang off a changing content height. The fill, because a
            // list too short to scroll never reaches its top edge. And staying
            // at the end: a row that grows *after* it was laid out - a reaction
            // appearing under the last message, a picture that finished
            // loading, an edit that got longer - pushes the end below the
            // viewport without changing the count, and the count is all the
            // tail logic above watches. Both belong in this one handler; a
            // second onContentHeightChanged in the same object is not an
            // addition, it is a load error that kills the page.
            onContentHeightChanged: {
                page.fillScreen()
                // A jump that is still looking for its row outranks the tail:
                // the rows of the very pagination it asked for keep growing
                // the content, and every one of those would drag the view back
                // to the end under it.
                if (page.followTail && page.jumpTargetId.length === 0) {
                    tailTimer.restart()
                }
            }

            // The viewport's own height changes under the list: the keyboard
            // takes the lower half of the screen, the pinned banner appears,
            // the composer grows with what is being typed. Qt keeps the
            // content's *top* where it is, so everything the eye was on slides
            // down out of sight - which is how "the message list does not move
            // when the keyboard opens, only the text field does" was reported.
            // Shrinking, the content is pushed down by exactly what was lost:
            // the bottom edge, and with it the newest message, stays where it
            // stood. Growing, whoever was following the tail is put back at the
            // end, because a view scrolled past its content is a page of empty
            // space that only a flick corrects.
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

            // An empty room and a room that is still loading look the same, so they
            // are told apart explicitly rather than showing "no messages" during
            // the first fetch.
            ViewPlaceholder {
                enabled: timelineView.count === 0 && !page.invited && matrix.timelineReady
                text: qsTr("No messages")
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
                // Never against a hand on the list. Reaching the top asks for
                // older messages, they arrive as rows, and the row count is
                // what tells the tail to follow - so the conversation jumped
                // to its end in the middle of the swipe that fetched them.
                // The flag only says where the view belongs when nobody is
                // moving it; `moving` covers the drag and the flick after it.
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
                        // The same step down as the face beside it. The two
                        // are offers; the send arrow is the action, and it is
                        // the one thing in this row that may brighten - which
                        // it does by itself the moment there is something to
                        // send.
                        icon.opacity: Theme.opacityLow
                        onClicked: pageStack.push(contentPicker)
                    }

                    TextArea {
                        id: messageField

                        anchors {
                            // Straight after the paper clip, and where that is
                            // hidden (while editing) at the page margin. The
                            // lock that used to stand in this gap says its
                            // piece at the top of the room now.
                            left: attachButton.visible ? attachButton.right
                                                       : parent.left
                            leftMargin: attachButton.visible
                                        ? 0 : Theme.horizontalPageMargin
                            right: emojiButton.left
                            verticalCenter: parent.verticalCenter
                        }
                        // The field brings a page margin of its own on both
                        // sides, and in a row that already has a button at
                        // each end it is margin twice over: it pushed the text
                        // a finger's width away from the lock and cut the line
                        // short of the face. The row's own spacing does that
                        // job here.
                        textMargin: 0
                        placeholderText: page.editingEventId.length > 0
                                         ? qsTr("New text")
                                         : qsTr("Message")
                        labelVisible: false

                        // Grow with the text, but stop before the composer eats the
                        // screen. Past the cap the field scrolls to the cursor
                        // internally, instead of pushing its first lines up behind
                        // the header where they can no longer be reached.
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

                    // Microphone and send share the right slot - they are never
                    // both there - so the face sits next to whichever of them
                    // is showing and does not move when the other takes over.
                    // Not an IconButton: the icon is drawn (FaceIcon), and only
                    // a theme icon can be handed to one of those.
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
                            // Measured against the theme's own icons in this
                            // row rather than set to the same nominal size:
                            // what the eye compares is the drawn figure, and
                            // the theme's icons fill their box to different
                            // degrees. At iconSizeMedium the face came out a
                            // third taller than the send arrow beside it -
                            // three buttons, three sizes. 0.78 puts its circle
                            // at the height of the paper clip's ink.
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
                        // `inputMethodComposing` is the uncommitted word, and
                        // it is what Qt 5.6 offers - `preeditText` arrives in
                        // 5.7 and is out of reach here.
                        enabled: messageField.text.trim().length > 0
                                 || messageField.inputMethodComposing
                        icon.source: "image://theme/icon-m-send"
                        // The arrow is a flat wide triangle and fills barely
                        // half the height its box has, which next to the clip
                        // reads as a smaller button rather than a different
                        // shape. Scaled, not replaced: the artwork is the
                        // platform's, and the alternative would be drawing an
                        // arrow of our own beside the platform's clip.
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
            // Not sent from here any more. Sending straight out of the picker
            // made three things impossible at once: a caption, an answer that
            // carries a picture, and changing one's mind after tapping the
            // wrong thumbnail. `replace` puts the send page where the picker
            // stood, so going back lands in the conversation and not in the
            // file list again.
            // Only remembered here, never navigated from here. The picker is
            // running its own transition at this moment - it answered a
            // `replace` with "cannot pop while transition is in progress" in
            // the device journal, and what stayed on screen afterwards was the
            // file list, not the send page. The timer below opens it once the
            // picker has left on its own.
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
                // What was typed goes along as the caption: text and picture
                // were meant as one message. The field is cleared so it cannot
                // be sent a second time, and filled again if the send is
                // called off - a wrong tap must not eat a written message.
                var typed = messageField.text
                messageField.text = ""
                pageStack.push(Qt.resolvedUrl("SendMediaPage.qml"), {
                    path: picked.path,
                    mimeType: picked.mimeType,
                    caption: typed,
                    replyTo: page.replyingEventId,
                    replySender: page.replyingTo,
                    replyBody: page.replyingBody,
                    // Handed back here rather than sent from the dialog: the
                    // warning about unverified recipients lives on this page,
                    // and it was the one thing an attachment went past. The
                    // word "hi" was held and the photograph was not.
                    send: function (path, mimeType, caption, replyTo) {
                        page.pendingAction = {
                            "kind": "sendMedia",
                            "path": path,
                            "mimeType": mimeType,
                            "caption": caption,
                            "replyTo": replyTo
                        }
                    },
                    // Only reached where this page does not take the send over
                    // itself; kept so the dialog still works for any other
                    // caller.
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
                // Stopped, not just reset: without this "give up" meant asking
                // the stack to pop every second and a half for as long as the
                // page lived.
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
        // The word being typed is not in `text` yet.
        //
        // Sailfish's keyboard keeps the current word in the input method's
        // preedit buffer and hands it over only when it is committed - a space,
        // a punctuation mark, a tapped suggestion. Whoever types a message and
        // reaches straight for send therefore sent everything but the last
        // word, and a single-word message went out empty. Reported from the
        // field for both sending and editing. Committing here covers every way
        // out, the warning dialog included, because they all pass through this
        // one function.
        Qt.inputMethod.commit()
        // In an encrypted room, warn once before the message reaches a
        // recipient whose devices were never verified. "Send anyway" (and an
        // optional "remember") is handled in the dialog; it then calls back.
        var pending = pendingUnverified()
        if (pending.length > 0) {
            var dialog = pageStack.push(Qt.resolvedUrl("UnverifiedRecipientsDialog.qml"),
                                        { users: pending })
            dialog.accepted.connect(page.doSubmit)
            return
        }
        doSubmit()
    }

    // The keyboard after a message has gone out. Asked for, and it is what the
    // big messengers do: what one wants to see once the message is away is the
    // conversation, not the field one has just emptied. The setting keeps it up
    // for whoever writes several in a row.
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

    /// An attachment, through the same gate a typed message goes through.
    ///
    /// What follows a send happens here, after the send: called off at the
    /// warning, the reply this attachment was an answer to is still there, and
    /// the conversation has not been scrolled anywhere.
    function submitMedia(action) {
        var pending = pendingUnverified()
        if (pending.length > 0) {
            var dialog = pageStack.push(Qt.resolvedUrl("UnverifiedRecipientsDialog.qml"),
                                        { users: pending })
            dialog.accepted.connect(function() { page.sendMediaNow(action) })
            return
        }
        page.sendMediaNow(action)
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

    // After an attachment went out as a reply. Unlike cancelReply this leaves
    // the composer alone — a half-typed message is not part of the picture
    // that was just sent.
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

    // Saving needs the original, not the thumbnail, so it is fetched first and
    // written once it arrives.
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

    // A message body is untrusted and is never rendered as markup. When it
    // visibly contains a web link, the whole body is HTML-escaped first and
    // only the detected URLs are wrapped in anchors — the StyledText that
    // shows the result renders nothing this function did not write itself.
    // Opens the thread of a message, starting one where there is none. The
    // landscape action page calls this too.
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

    // Which message's readers are unfolded, and their names as one line. Only
    // ever one at a time: the names are an aside, not a second conversation.
    // Names under a message, unfolded on demand: who has read up to here (the
    // eye), or who set one reaction (a chip held down). One line and one state
    // for both - a second line would mean a second label in every row of the
    // conversation, and the rows are what this page pays for.
    property string namesEventId: ""
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

    /// Tap on the "read by" mark. Asks the core for the names - the rows carry
    /// only the count - and folds them away again on a second tap.
    function showReaders(eventId) {
        if (page.namesEventId === eventId && page.namesKey.length === 0) {
            page.clearNames()
            return
        }
        page.clearNames()
        page.namesEventId = eventId
        matrix.loadReaders(eventId)
    }

    /// Press and hold on a reaction: who set it. A tap keeps doing what a chip
    /// is for, which is adding or taking back one's own. The key that goes to
    /// the core is the one the row sends with, not the one it draws.
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
            // Nobody to name: fold the line away again instead of leaving the
            // reaction standing on its own, which reads as a line that failed
            // to load rather than as an answer.
            if (who.length === 0) {
                page.clearNames()
                return
            }
            page.namesText = page.namesPrefix + who.join(", ")
        }
    }

    // The face next to the send button opens the same page the reaction uses,
    // in the mode that hands the character back instead of sending it. It goes
    // where the cursor is, not at the end: an emoji belongs mid-sentence as
    // often as at its end.
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
            // Only the two schemes this app ever produces. Everything the
            // system would otherwise dispatch - file:, tel:, sms:, whatever
            // else is registered on the device - is a stranger's choice, not
            // the user's, and the allowlist belongs at the point of no return.
            if (/^https?:\/\//i.test(link)) {
                Qt.openUrlExternally(link)
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

    /// A URL inside an href, where Qt decodes no entities: the ampersand has
    /// to stay itself or the opened address is not the one that was shown.
    /// Everything that could end the attribute is refused instead.
    function safeHref(url) {
        return /["'<>`\\\s]/.test(url) ? "" : url
    }

    function linkifyBody(body) {
        var escaped = page.escapeBody(body)
        // One pass over both kinds, web address first: a matrix.to permalink
        // carries a room address inside itself, and matching that separately
        // would cut the link in half.
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
                // A Matrix permalink is handled in the app, so it stays
                // tappable even where web links are switched off - it never
                // reaches a browser.
                if (!settings.clickableLinks && !MatrixLinks.parse(target)) {
                    return target + trail
                }
                var href = page.safeHref(target)
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

    /// A short word over the conversation. Used where a tap does nothing on
    /// purpose: silence is indistinguishable from a tap that never registered,
    /// and the user tries again instead of learning why.
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
