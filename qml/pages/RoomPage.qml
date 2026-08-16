import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import QtMultimedia 5.6

// A single room: history above, composer below.
//
// The list keeps the core's order — oldest first — and follows the bottom
// unless the user has scrolled up to read.
Page {
    id: page

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
        }

        if (status === PageStatus.Active && !invited) {
            // Returning from the pinned view the shared timeline must show
            // live events again. For a timeline that is already live this is
            // a no-op.
            matrix.openRoom(roomId)
            matrix.markRead()
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

    // Asks this page to scroll to a message; called by the pinned overview
    // before it pops back.
    function jumpToPinned(eventId) {
        jumpTargetId = eventId
        jumpPagesLeft = 10
    }

    function tryJump() {
        if (jumpTargetId.length === 0) {
            return
        }
        var idx = matrix.timeline.indexOfEvent(jumpTargetId)
        if (idx >= 0) {
            jumpTargetId = ""
            followTail = false
            timelineView.positionViewAtIndex(idx, ListView.Center)
        } else if (jumpPagesLeft > 0 && !matrix.timelineAtStart) {
            // Not loaded yet: fetch older history and retry when it arrives.
            jumpPagesLeft--
            matrix.loadOlder()
        } else {
            jumpTargetId = ""
        }
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

            // Everything about the room rather than about the running
            // conversation lives one page further in: members, pinned
            // messages, inviting, encryption, the room's own details. This
            // menu had grown to ten entries, which is barely draggable on a
            // small screen in landscape. Last in the list, so the short tug
            // reaches it.
            MenuItem {
                text: qsTr("Room info")
                onClicked: pageStack.push(Qt.resolvedUrl("RoomInfoPage.qml"), {
                                              roomId: page.roomId,
                                              roomName: page.roomName,
                                              invited: page.invited
                                          })
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
                readonly property bool hasPreview: isImage
                                                   || (isVideo && !!model.media.thumbnailSource)

                width: timelineView.width
                contentHeight: isBubble
                               ? bubble.height + Theme.paddingMedium
                               : (model.kind === "date"
                                  ? dayLabel.height + Theme.paddingLarge
                                  : (isSystem ? systemLabel.height + Theme.paddingLarge : 0))

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
                        text: qsTr("Delete")
                        visible: row.isOwn && !page.isLandscape
                        onClicked: matrix.deleteMessage(model.eventId)
                    }

                    MenuItem {
                        text: qsTr("More…")
                        visible: page.isLandscape
                        onClicked: pageStack.push(Qt.resolvedUrl("MessageActionsPage.qml"), {
                                                      roomPage: page,
                                                      eventId: model.eventId || "",
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
                        Row {
                            id: replyQuote

                            // No explicit width: like every sibling, the row
                            // is as wide as its content and each child sizes
                            // bottom-up. An earlier form derived a width from
                            // the children's implicit widths while the
                            // children read it back — the mixed directions
                            // the width rule forbids, and the device journal
                            // duly reported the loop.
                            visible: !!model.replyTo
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

                            Column {
                                id: quoteTexts

                                // What the labels may use once the bar and
                                // the spacing took their share.
                                readonly property real maxWidth: bubbleColumn.maxTextWidth
                                                                 - Theme.paddingSmall * 1.5

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
                                    text: model.replyTo ? (model.replyTo.sender || "") : ""
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
                                    color: Theme.secondaryColor
                                    maximumLineCount: 2
                                    wrapMode: Text.Wrap
                                    elide: Text.ElideRight
                                    textFormat: Text.PlainText
                                    // The ellipsis stands in while the quoted
                                    // message is being fetched — and stays
                                    // for one that has no text, a redacted
                                    // original say.
                                    text: model.replyTo ? (model.replyTo.body || "…") : ""
                                }
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
                            visible: !row.hasPreview && !row.isAudio

                            Label {
                                id: bodyMeasure

                                visible: false
                                textFormat: Text.PlainText
                                font.pixelSize: bodyLabel.font.pixelSize
                                font.italic: bodyLabel.font.italic
                                text: bodyLabel.text
                            }
                            // Plain text on purpose: a message body is untrusted,
                            // and AutoText would render anything that looks like
                            // HTML — including <img> that phones home.
                            textFormat: Text.PlainText
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
                                if (model.kind === "undecryptable") {
                                    return qsTr("Cannot be decrypted — this device is missing the key")
                                }
                                if (model.kind === "redacted") {
                                    return qsTr("Message deleted")
                                }
                                if (row.isFile) {
                                    return "📎 " + (model.body || "")
                                }
                                return model.body || ""
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
                                  : ""
                        }
                    }
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
                }
            }

            onMovementEnded: page.followTail = atYEnd

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
            onContentHeightChanged: page.fillScreen()

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
                            right: recordButton.visible ? recordButton.left : sendButton.left
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
                            right: sendButton.left
                            rightMargin: Theme.paddingMedium
                            verticalCenter: parent.verticalCenter
                        }
                        visible: page.editingEventId.length === 0
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
            onSelectedContentPropertiesChanged: {
                matrix.sendMedia(selectedContentProperties.filePath,
                                 selectedContentProperties.mimeType)
                page.followTail = true
            }
        }
    }

    // Recipients with unverified devices the user has not chosen to trust yet.
    // Empty means nothing stands in the way of sending.
    function pendingUnverified() {
        var out = []
        for (var i = 0; i < unverifiedUsers.length; i++) {
            var entry = unverifiedUsers[i]
            if (!matrix.isRecipientTrusted(entry.userId)) {
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
        page.editingEventId = ""
        messageField.forceActiveFocus()
    }

    function cancelReply() {
        page.replyingEventId = ""
        page.replyingTo = ""
        messageField.text = ""
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
}
