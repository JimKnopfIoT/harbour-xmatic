import QtQuick 2.0
import Sailfish.Silica 1.0
import "MatrixLinks.js" as MatrixLinks
import "LinkMarks.js" as LinkMarks
import "MessageBody.js" as MessageBody
import "Composing.js" as Composing

// One thread of a room: root first, replies below, own composer. Text-focused,
// attachments render as their kind.
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

    /// The line a message carries when its authenticity is in doubt, same wording
    /// as the room: a thread must not say less about a message.
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

    function doSubmit() {
        matrix.sendThreadMessage(composer.text, composer.mentionIds())
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

    // A tapped link, as in the room below: a Matrix address is answered inside
    // the app, anything else is a web address.
    function followLink(link) {
        var target = MatrixLinks.decide(link)
        if (target.kind === "none") {
            return
        }
        if (target.kind === "web") {
            // The address before it is opened: what a link says and where it
            // goes are two strings a stranger writes.
            var dialog = pageStack.push(Qt.resolvedUrl("ConfirmDialog.qml"), {
                                            question: qsTr("Open this address?"),
                                            subject: link,
                                            acceptLabel: qsTr("Open"),
                                            plainCancel: true
                                        })
            dialog.accepted.connect(function() { Qt.openUrlExternally(link) })
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

        delegate: ListItem {
            id: threadRow

            // "system" covers calls, membership and profile changes: without a row of
            // their own a thread made of those renders as an empty page.
            readonly property bool isSystem: model.kind === "system"
            // The link a press is holding, while its menu is open; the room's link menu.
            property string heldLink: ""
            property var linkMarks: []

            function holdLink(link, x, y) {
                var origin = bodyText.mapToItem(threadRow, 0, 0)
                linkHighlight.x = origin.x
                linkHighlight.y = origin.y
                threadRow.heldLink = link
                // From the preview card there is no press in the text, and nothing to mark.
                threadRow.linkMarks = x === undefined ? []
                                                  : LinkMarks.sweep(bodyText, threadRow, link, x, y)
                threadRow.openMenu()
            }

            width: threadView.width
            enabled: model.kind === "message"
            showMenuOnPressAndHold: false
            _showPress: false
            onClicked: {
                var link = LinkMarks.linkAt(bodyText, threadRow, mouse.x, mouse.y)
                if (link.length > 0) {
                    threadRow.holdLink(link, mouse.x, mouse.y)
                }
            }
            onPressAndHold: {
                var link = LinkMarks.linkAt(bodyText, threadRow, mouse.x, mouse.y)
                if (link.length > 0 && threadRow.down) {
                    threadRow.holdLink(link, mouse.x, mouse.y)
                }
            }
            onMenuOpenChanged: {
                if (!menuOpen) {
                    threadRow.heldLink = ""
                    threadRow.linkMarks = []
                }
            }

            menu: ContextMenu {
                MenuItem {
                    text: qsTr("Open link?")
                    onClicked: page.followLink(threadRow.heldLink)
                }

                MenuItem {
                    text: qsTr("Copy link")
                    onClicked: Clipboard.text = threadRow.heldLink
                }

                MenuItem {
                    text: qsTr("Forward link")
                    onClicked: pageStack.push(Qt.resolvedUrl("ForwardPage.qml"),
                                              { body: threadRow.heldLink })
                }

                MenuItem {
                    text: qsTr("Pin")
                    visible: (model.eventId || "").length > 0
                             && matrix.roomPermissions.pin !== false
                    onClicked: matrix.pinMessage(model.eventId, true)
                }
            }

            contentHeight: {
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
                // A held link is marked alone: the post steps back with the rest of the page.
                opacity: threadRow.linkMarks.length > 0 ? Theme.opacityLow : 1.0

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
                    id: bodyText

                    width: parent.width
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    font.italic: model.kind !== "message"
                    color: model.kind === "message" ? Theme.primaryColor
                                                    : Theme.secondaryColor
                    // The room's own rule and settings: links, emoji pictures, the core's markup.
                    readonly property string richBody: model.kind === "message" && !model.media
                                                       ? MessageBody.rich(model.body, model.formatted,
                                                                          settings,
                                                                          Theme.highlightColor,
                                                                          Math.round(Theme.fontSizeSmall * 1.3),
                                                                          function(key) { return matrix.emojiSource(key) })
                                                       : ""
                    textFormat: richBody.length > 0 ? Text.StyledText : Text.PlainText
                    linkColor: Theme.highlightColor
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
                        if (richBody.length > 0) {
                            return richBody
                        }
                        return model.body || ""
                    }
                }

                // The linked page, under the same Privacy setting as in the room.
                Loader {
                    id: previewLoader

                    readonly property string url: model.kind === "message" && !model.media
                                                  && !model.poll && !model.location
                                                  ? MessageBody.previewUrl(model.body,
                                                                           settings.linkPreviews,
                                                                           page.encrypted
                                                                           || !page.encryptionKnown)
                                                  : ""
                    active: url.length > 0
                    visible: active && !!item && item.available
                    sourceComponent: LinkPreviewCard {
                        availableWidth: rowColumn.width
                        url: previewLoader.url
                        tappable: settings.clickableLinks
                        onActivated: threadRow.holdLink(link)
                    }
                }

                // What the SDK will not vouch for: red is not what it claims to be, grey
                // cannot be checked. Silent otherwise.
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

            LinkHighlight {
                id: linkHighlight

                label: bodyText
                marks: threadRow.linkMarks
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

    // Sending on the thread timeline threads automatically. The same composer
    // the room below uses, without the offers a thread has no room for.
    Composer {
        id: composer

        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }

        roomId: page.roomId
        placeholderText: qsTr("Reply in thread")
        onSubmitted: page.submit()
    }
}
