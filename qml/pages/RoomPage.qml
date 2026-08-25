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

    // Set while an already sent message is being rewritten.
    property string editingEventId: ""
    // Set while a reply is being composed.
    property string replyingEventId: ""
    /// The quoted message's text, for the quote on the send page.
    property string replyingBody: ""
    property string replyingTo: ""

    allowedOrientations: Orientation.All

    // Whether new messages should scroll the view along.
    property bool followTail: true

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
            matrix.openRoom(roomId)
            refreshRecipients()
        }
    }

    // The warning only makes sense for an encrypted room; an unencrypted one
    // has no device trust to check. The core answers with recipientsChecked.
    function refreshRecipients() {
        if (page.encrypted) {
            matrix.checkRecipients(page.roomId)
        }
    }

    // Where reading stopped last time, until the row it names is on screen.
    // Not hunted for through the history like a permalink jump: if it is not
    // in the loaded window, the room simply opens at the newest message.
    property string unreadFromId: ""

    // Once per built timeline. The page stays alive while a sub-page is on top,
    // and coming back re-opens the same one - the position must not jump then.
    // A timeline that was actually rebuilt is a different matter: the rows are
    // new, the view starts from nothing, and where reading stopped is exactly
    // where it belongs. That is the way back from a room opened over this one.
    property bool unreadHandled: false

    Connections {
        target: matrix

        onTimelineOpened: {
            if (rebuilt) {
                page.unreadHandled = false
            }
            if (page.unreadHandled) {
                return
            }
            page.unreadHandled = true
            page.unreadFromId = readMarker
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
            if (page.followTail && page.status === PageStatus.Active
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
            } else if (page.followTail || timelineView.atYEnd) {
                matrix.markRead()
            }
        }
    }

    // Opens at the first message that arrived after this device last read the
    // room. Silent when the marker is the newest row - then there is nothing
    // unread and the view belongs at the bottom, where it already is.
    function showFirstUnread() {
        var marker = page.unreadFromId
        page.unreadFromId = ""
        if (marker.length === 0 || page.jumpTargetId.length > 0) {
            return
        }
        var idx = matrix.timeline.indexOfEvent(marker)
        if (idx < 0 || idx >= matrix.timeline.count - 1) {
            return
        }
        page.followTail = false
        timelineView.positionViewAtIndex(idx + 1, ListView.Beginning)
        // What "at the end" means is only known once the view has settled.
        // Two unread messages fit on the screen, so the jump lands at the end
        // and the room is being read; a longer run does not, and it is not.
        // Without asking, the room stayed parked: new messages no longer
        // followed, and nothing counted as read either.
        followCheck.restart()
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
        } else if (status === PageStatus.Inactive) {
            matrix.setVisibleRoom("")
            // Leaving is the last moment to say the room was read. The timer
            // is debounced by nearly a second, and stepping out inside that
            // second used to drop the receipt - the room then stood in the
            // list as unread although every message had been on screen.
            if (!invited && (page.followTail || timelineView.atYEnd)) {
                matrix.markRead()
            }
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
        tryJump()
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
        jumpNoticeTimer.restart()
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
    /// Row count when the last automatic round was started, -1 for none yet.
    property int fillLastCount: -1

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
        if (page.fillLastCount >= 0 && timelineView.count <= page.fillLastCount) {
            if (++page.fillEmptyRounds >= page.fillEmptyLimit) {
                return
            }
        } else {
            page.fillEmptyRounds = 0
        }

        page.fillLastCount = timelineView.count
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
            page.fillLastCount = -1
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

                Label {
                    width: parent.width
                    horizontalAlignment: Text.AlignRight
                    text: page.roomName
                    font.pixelSize: Theme.fontSizeLarge
                    font.family: Theme.fontFamilyHeading
                    color: roomHeader.highlighted ? Theme.highlightColor
                                                  : Theme.primaryColor
                    truncationMode: TruncationMode.Fade
                    // A room name with a newline in it would push the strip open
                    // and take the space the conversation needs.
                    wrapMode: Text.NoWrap
                    maximumLineCount: 1
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
                    // Being cut off from the server outranks anything about the
                    // room itself: a silently stale conversation is the one
                    // thing the user cannot see for themselves.
                    text: matrix.syncState === "offline"
                          ? qsTr("Offline — waiting for the network")
                          : page.invited
                            ? qsTr("Invitation")
                            : page.encrypted ? qsTr("End-to-end encrypted")
                                             : qsTr("Not encrypted")
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
                readonly property bool sanePicture: !model.media
                        || (((model.media.width || 0) <= 8192
                             && (model.media.height || 0) <= 8192)
                            // The declared size, before anything is fetched:
                            // the ceiling in the core only sees bytes that are
                            // already in memory.
                            && (model.media.size || 0) <= 100 * 1024 * 1024)

                readonly property bool hasPreview: sanePicture
                                                   && (isImage
                                                       || (isVideo && !!model.media.thumbnailSource))

                width: timelineView.width
                contentHeight: isBubble
                               ? bubble.height + Theme.paddingMedium
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
                                                                   : model.eventId
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
                        visible: model.kind === "message" && (model.eventId || "").length > 0
                                 && !page.isLandscape
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
                        onClicked: matrix.deleteMessage(model.eventId || "",
                                                        model.txnId || "")
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
                    size: Theme.iconSizeSmall
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
                                    + Theme.iconSizeSmall + Theme.paddingSmall
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
                                                    - (row.isOwn ? 0 : Theme.iconSizeSmall
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

                        Label {
                            id: senderLabel

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
                            visible: !!model.replyTo
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
                                            matrix.requestMedia(mediaKey, quotedMedia.thumbnailSource, false)
                                        } else {
                                            matrix.requestMedia(mediaKey, quotedMedia.source, true)
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
                                    matrix.requestMedia(model.id, model.media.thumbnailSource, false)
                                } else {
                                    matrix.requestMedia(model.id, model.media.source, true)
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
                                running: attachment.visible && attachment.source == ""
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
                                // Only for a video without a preview; with one the
                                // picture itself is the target.
                                enabled: row.isVideo && !row.hasPreview
                                onClicked: page.openVideo(model.id, model.media)
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
                                                                : model.threadRoot
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
                                        onClicked: matrix.toggleReaction(row.rowEventId,
                                                                         modelData.sendKey
                                                                         || modelData.key)

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
                                                size: Theme.iconSizeExtraSmall
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

                        Label {
                            id: metaLabel

                            // Right-aligned inside whatever the bubble ended up
                            // being, so it reads from the widest sibling.
                            width: Math.max(bodyLabel.visible ? bodyLabel.width : 0,
                                            attachment.width,
                                            audioRow.visible ? audioRow.width : 0,
                                            senderLabel.visible ? senderLabel.width : 0,
                                            replyQuote.visible ? replyQuote.width : 0,
                                            threadLabel.visible ? threadLabel.width : 0,
                                            implicitWidth)
                            horizontalAlignment: Text.AlignRight
                            font.pixelSize: Theme.fontSizeTiny
                            // A message that did not get out is the one thing in
                            // this line worth a colour: half opacity alone reads
                            // as "still going" and let someone send the same text
                            // a second time.
                            color: model.sendState === "failed"
                                   ? Theme.errorColor : Theme.secondaryColor
                            // Only messages carry a timestamp; the shared delegate
                            // instantiates this label for every row regardless.
                            text: row.isBubble
                                  ? (matrix.pinnedEventIds.indexOf(model.eventId) >= 0 ? "📌 " : "")
                                    + (model.sendState === "failed"
                                       ? qsTr("not sent") + " · " : "")
                                    + (model.edited === true ? qsTr("edited") + " · " : "")
                                    + Format.formatDate(new Date(model.timestamp), Formatter.TimeValue)
                                    // On the newest own message the others
                                    // have read up to, which is where a
                                    // receipt actually means something - it
                                    // usually hangs on their own latest
                                    // message, not on ours. Empty while the
                                    // status is switched off: nothing is
                                    // tracked then.
                                    + (model.readMark === true && model.readMarkBy > 0
                                       ? " · " + qsTr("read by %n", "", model.readMarkBy) : "")
                                  : ""

                            // Only where there is something to unfold, so the
                            // long press on every other message still belongs
                            // to the message menu.
                            MouseArea {
                                anchors.fill: parent
                                enabled: model.readMark === true && model.readMarkBy > 0
                                onPressAndHold: page.showReaders(model.eventId)
                            }
                        }

                        // Who they are, as one line rather than a list: a name
                        // per row would push the conversation off the screen
                        // for an aside.
                        Label {
                            visible: page.readersEventId === model.eventId
                                     && page.readerNames.length > 0
                            width: metaLabel.width
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
                            text: page.readerNames
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
                if (page.followTail) {
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
            onTriggered: timelineView.positionViewAtEnd()
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
                        onClicked: pageStack.push(contentPicker)
                    }

                    TextArea {
                        id: messageField

                        anchors {
                            left: attachButton.visible ? attachButton.right : parent.left
                            right: emojiButton.left
                            verticalCenter: parent.verticalCenter
                        }
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
                            rightMargin: Theme.paddingMedium
                            verticalCenter: parent.verticalCenter
                        }
                        width: Theme.itemSizeSmall
                        height: Theme.itemSizeSmall

                        onClicked: page.pickEmoji()

                        FaceIcon {
                            anchors.centerIn: parent
                            size: Theme.iconSizeMedium
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
                        enabled: messageField.text.trim().length > 0
                        icon.source: "image://theme/icon-m-send"
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

    function doSubmit() {
        if (page.editingEventId.length > 0) {
            matrix.editMessage(page.editingEventId, messageField.text)
            page.cancelEdit()
            return
        }
        if (page.replyingEventId.length > 0) {
            matrix.replyToMessage(page.replyingEventId, messageField.text)
            page.cancelReply()
            page.followTail = true
            return
        }
        matrix.sendMessage(messageField.text)
        messageField.text = ""
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
        matrix.requestMedia(key, media.source, false)
    }

    function openVideo(itemId, media) {
        var key = itemId + "/full"
        var known = matrix.mediaPath(key)
        matrix.requestMedia(key, media.source, false)
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
        matrix.requestMedia(key, item.media.source, false)
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
                           rootEventId: eventId
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
    property string readersEventId: ""
    property string readerNames: ""

    /// Long press on the "read by" line. Asks the core for the names - the
    /// rows carry only the count - and folds them away again on a second press.
    function showReaders(eventId) {
        if (page.readersEventId === eventId) {
            page.readersEventId = ""
            page.readerNames = ""
            return
        }
        page.readersEventId = eventId
        page.readerNames = ""
        matrix.loadReaders(eventId)
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
            if (eventId !== page.readersEventId) {
                return
            }
            var names = []
            for (var i = 0; i < readers.length; i++) {
                names.push(readers[i].name)
            }
            page.readerNames = names.join(", ")
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
        matrix.requestMedia(key, media.source, false)
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

    // Said out loud when a jump gives up. Silence was the old behaviour and it
    // is indistinguishable from a tap that never registered — the user tries
    // again instead of learning that the message is beyond reach.
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
            text: qsTr("That message is not in the loaded history")
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
