import QtQuick 2.5
import Sailfish.Silica 1.0
import QtMultimedia 5.6
import "LinkMarks.js" as LinkMarks
import "MessageBody.js" as MessageBody
import "Strings.js" as Strings

// One timeline row: message, system line or date. Everything page-wide goes
// through `host`, so the room and a thread draw the same row.
ListItem {
    id: row

    /// The page holding the list: its state and the actions a row starts.
    property Item host
    /// The list the row sits in; its width bounds the bubble.
    property Item listView
    /// The page's one voice player.
    property var player
    /// Media, reactions, names and shield lines, from the host.
    readonly property var actions: host ? host.timelineActions : null
    /// The row's media cache key: two timelines number their rows independently.
    readonly property string mediaId: (actions ? actions.mediaPrefix : "") + model.id

    // Undecryptable and deleted events get a bubble too, so a gap in
    // the conversation is visible rather than silently skipped.
    readonly property bool isBubble: model.kind === "message"
                                     || model.kind === "undecryptable"
                                     || model.kind === "redacted"
    // The last message read before this visit carries the line. Not the SDK's
    // marker row: that needs receipt tracking and it moves as the room is read.
    readonly property bool showMarker: rowEventId.length > 0
                                       && rowEventId === host.markerEventId
                                       && index < row.listView.count - 1
    // The text an attachment was sent with. Empty for one sent
    // without, where `body` holds the file name instead.
    readonly property string rowEventId: model.eventId || ""
    // Same sender as the message above, same day: no picture, no name. The
    // line where the new messages start ends a run.
    readonly property bool continues: (model.continuesFrom || "").length > 0
                                      && model.continuesFrom !== host.markerEventId
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
    // Offered where the switch is on; whether it can work is the check's
    // business on the privacy page, and the answer's in the bubble.
    readonly property bool canTranscribe: row.isAudio && settings.voiceTranscripts
                                          && row.rowEventId.length > 0
    readonly property string transcriptState: matrix.transcripts.revision >= 0
            ? matrix.transcripts.state(row.rowEventId) : ""
    readonly property bool isVideo: model.kind === "message"
                                    && model.msgtype === "m.video"
                                    && !!model.media
    // A picture whose kind this device can animate. Asked of Qt rather
    // than listed here: the WebP handler on this Qt reads a file but
    // does not animate it, and a mark the viewer cannot honour is a lie.
    // Only the mark is drawn here; it plays in the viewer, when asked.
    /// The sender's own figure, where there is one. Not every client
    /// declares it, and none is written rather than guessed.
    readonly property string mediaSizeText:
        (row.isVideo || row.isFile || row.isAudio || row.isImage)
        && model.media && model.media.size > 0
        ? Format.formatFileSize(model.media.size) : ""

    readonly property bool isAnimated: row.isImage
            && host.animatableFormats.indexOf(
                String(model.media.mimetype || "").toLowerCase()
                    .replace(/^image\//, "")) >= 0
    readonly property bool isFile: model.kind === "message"
                                   && !!model.media
                                   && !isImage
    // The first web address in the text, where the setting allows a
    // preview here. Empty means no card - and no question to the server.
    readonly property string previewUrl: {
        if (model.kind !== "message" || !!model.media || row.isPoll
                || row.isLocation) {
            return ""
        }
        // Unknown counts as encrypted: the failure direction of this
        // gate is the one that asks the server nothing.
        return MessageBody.previewUrl(model.body, settings.linkPreviews,
                                      host.encrypted || !host.encryptionKnown)
    }
    // A poll draws itself; its fallback text would repeat the
    // question and list the answers a second time.
    readonly property bool isPoll: model.kind === "message" && !!model.poll
    // A location draws itself; its fallback is a geo: URI.
    readonly property bool isLocation: model.kind === "message" && !!model.location
    readonly property bool isOwn: model.own === true
    // A sent message of someone else's, where the room allows it.
    readonly property bool canDeleteForOther: !row.isOwn
            && host.canRedactOthers
            && (model.kind === "message" || model.kind === "undecryptable")
            && (model.eventId || "").length > 0
    // An undecryptable row's one action: spam in an encrypted room reads like this.
    readonly property bool hasMenu: model.kind === "message"
            || (model.kind === "undecryptable" && (row.isOwn || row.canDeleteForOther))
    // The body as markup, or empty where plain text will do. One property for
    // format and text, so a Label can never parse what was not built for it.
    readonly property int emojiPixels: Math.round(Theme.fontSizeSmall * 1.3)
    // A file row: only its caption.
    readonly property string richBody: model.kind !== "message"
                                       || (!!model.media && !row.hasCaption)
                                       ? ""
                                       : MessageBody.rich(!!model.media ? row.captionText
                                                                        : model.body,
                                                          model.formatted,
                                                          settings,
                                                          Theme.highlightColor,
                                                          emojiPixels,
                                                          function(key) { return matrix.emojiSource(key) })

    // Calls and membership changes show as a centred line so a room made of them
    // is not empty. What the behaviour page hides collapses but stays, or indices drift.
    readonly property bool isSystem: model.kind === "system"
                                     && host.systemShown(model.system)

    // Localised here, not in the core: the token comes from Rust, the
    // wording and language belong to the UI.
    function systemText() {
        // The person the change is about, and the one who made it. For a ban
        // those are two different people, and naming only one named the wrong.
        var who = (model.name && model.name.length > 0)
                  ? model.name : model.senderName
        var by = model.senderName
        var line = ""
        var before = (model.previousName && model.previousName.length > 0)
                     ? model.previousName : model.sender
        switch (model.system) {
        case "call":            return qsTr("Call", "timeline system line, a noun")
        case "member.joined":   line = qsTr("%1 joined").arg(who); break
        case "member.left":     line = qsTr("%1 left").arg(who); break
        // One pass over the template: a name that reads "%2" stays a name.
        case "member.invited":  line = Strings.fill(qsTr("%1 invited %2"), [by, who]); break
        case "member.kicked":   line = Strings.fill(qsTr("%1 removed %2"), [by, who]); break
        case "member.banned":   line = Strings.fill(qsTr("%1 banned %2"), [by, who]); break
        case "member.unbanned": line = Strings.fill(qsTr("%1 unbanned %2"), [by, who]); break
        case "member.revoked":  line = Strings.fill(qsTr("%1 withdrew the invitation for %2"), [by, who]); break
        case "member.declined": line = qsTr("%1 declined the invitation").arg(who); break
        case "member.knocked":  line = qsTr("%1 asked to join").arg(who); break
        case "member":          line = qsTr("%1 changed membership").arg(who); break
        case "profile.name":
        case "profile.both":
            // Both changed: said as far as the switches show it.
            var nameShown = model.system === "profile.name"
                            || behaviour.showDisplayNameChanges
            var pictureShown = model.system === "profile.both"
                               && behaviour.showAvatarChanges
            var named = model.name && model.name.length > 0
            if (!nameShown) {
                line = qsTr("%1 changed their profile picture").arg(by)
            } else if (pictureShown) {
                line = named
                       ? Strings.fill(qsTr("%1 changed their display name to %2 and their profile picture"),
                                      [before, model.name])
                       : qsTr("%1 removed their display name and changed their profile picture").arg(before)
            } else {
                line = named
                       ? Strings.fill(qsTr("%1 changed their display name to %2"),
                                      [before, model.name])
                       : qsTr("%1 removed their display name").arg(before)
            }
            break
        case "profile.avatar":  line = qsTr("%1 changed their profile picture").arg(by); break
        default:                return ""
        }
        // Only where the core vouches for one - see member_reason().
        if (model.reason && model.reason.length > 0) {
            line = Strings.fill(qsTr("%1: %2", "system line and the reason given for it"),
                                [line, model.reason])
        }
        return line
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

    // Taller than the page: Silica's menu renders the row into one texture, and past
    // the GPU's limit that texture is black. Such a row gets the actions as a host.
    readonly property bool tall: contentHeight > host.height
    // The link a press is holding, while its menu is open.
    property string heldLink: ""
    property var linkMarks: []
    // Folded to three lines, past a page of text, until opened.
    readonly property bool expanded: host.expandedRevision >= 0
                                     && !!host.expandedRows[model.id]
    readonly property bool longBody: bodyFull.implicitHeight > host.height

    // Tap and hold alike. Too tall for a menu: the old confirmation instead.
    function holdLink(link, x, y) {
        if (row.tall) {
            host.followLink(link)
            return
        }
        var origin = bodyLabel.mapToItem(row, 0, 0)
        linkHighlight.x = origin.x
        linkHighlight.y = origin.y
        row.heldLink = link
        // From the preview card there is no press in the text, and nothing to mark.
        row.linkMarks = x === undefined ? []
                                          : LinkMarks.sweep(bodyLabel, row, link, x, y)
        row.openMenu()
    }

    function holdMenu() {
        if (!row.hasMenu) {
            return
        }
        if (row.tall) {
            row.showActions()
        } else {
            row.openMenu()
        }
    }

    // What an action needs of the row, copied out: the model row is gone once it leaves
    // the cache. `id` is the media key, the one every picture path uses.
    function attachmentItem() {
        return {
            "id": row.mediaId,
            "msgtype": model.msgtype,
            "media": model.media,
            "eventId": model.eventId || "",
            "threadRoot": model.threadRoot || "",
            "threadCount": model.threadCount || 0
        }
    }

    function showActions() {
        pageStack.push(Qt.resolvedUrl("MessageActionsPage.qml"), {
                           roomPage: row.host,
                           eventId: model.eventId || "",
                           txnId: model.txnId || "",
                           unsent: model.sendState === "failed",
                           body: model.body || "",
                           senderName: model.senderName || "",
                           isOwn: row.isOwn,
                           canDelete: row.isOwn || row.canDeleteForOther,
                           // Not in the live view an edit needs.
                           editable: model.editable === true && !host.inSlice,
                           isImage: row.isImage,
                           canSave: row.isFile || row.isImage,
                           canTranscribe: row.canTranscribe
                                          && row.transcriptState !== "working"
                                          && row.transcriptState !== "done",
                           item: row.attachmentItem()
                       })
    }

    readonly property bool hasPreview: sanePicture
                                       && (isImage
                                           || (isVideo && !!model.media.thumbnailSource))

    width: row.listView.width
    contentHeight: (isBubble
                    ? Math.max(bubble.height,
                               row.isOwn || row.continues ? 0 : host.avatarSize)
                      + Theme.paddingMedium
                    : (model.kind === "date"
                       ? dayLabel.height + Theme.paddingLarge
                       : (isSystem
                          ? Math.max(systemLabel.height,
                                     host.systemAvatarSize)
                            + Theme.paddingLarge
                          : 0)))
                   // The gap the line sits in. Any row can carry it, so it is added to the row's
                   // own height rather than being a row of its own.
                   + (showMarker ? Theme.paddingLarge : 0)

    // Only real messages react; dividers are not something to press.
    enabled: model.kind === "message" || row.isSystem
    // Every entry of the menu is about a message; a system line would
    // open it empty.
    showMenuOnPressAndHold: false
    // A link answers itself; the text around it opens the message's menu.
    onClicked: {
        var link = LinkMarks.linkAt(bodyLabel, row, mouse.x, mouse.y)
        if (link.length > 0) {
            row.holdLink(link, mouse.x, mouse.y)
        }
    }
    onPressAndHold: {
        if (!row.down) {
            return
        }
        var link = model.kind === "message"
                ? LinkMarks.linkAt(bodyLabel, row, mouse.x, mouse.y) : ""
        if (link.length > 0) {
            row.holdLink(link, mouse.x, mouse.y)
            return
        }
        row.holdMenu()
    }
    _showPress: false

    // A growing height under an open menu is the menu, not new rows:
    // following it scrolls the pressed row off the top.
    onMenuOpenChanged: {
        host.openMenus = Math.max(0, host.openMenus + (menuOpen ? 1 : -1))
        if (!menuOpen) {
            row.heldLink = ""
            row.linkMarks = []
        }
    }
    // A recycled delegate takes its menu with it, and that closing
    // does not always come back as a `menuOpen` change.
    Component.onDestruction: {
        if (menuOpen) {
            host.openMenus = Math.max(0, host.openMenus - 1)
        }
    }

    // `!host.isLandscape` on most entries: there the conversation keeps under
    // 600 px, four menu rows, and a message's menu needs six. Those move to a host.
    menu: ContextMenu {

        // A held link: its three actions and Pin; nothing else of the message.
        MenuItem {
            text: qsTr("Open link?")
            visible: row.heldLink.length > 0
            onClicked: host.followLink(row.heldLink)
        }

        MenuItem {
            text: qsTr("Copy link")
            visible: row.heldLink.length > 0
            onClicked: Clipboard.text = row.heldLink
        }

        MenuItem {
            text: qsTr("Forward link")
            visible: row.heldLink.length > 0
            onClicked: pageStack.push(Qt.resolvedUrl("ForwardPage.qml"),
                                      { body: row.heldLink })
        }

        MenuItem {
            // Also the only way out for a send that failed for good: it has no event id,
            // and without this it sits in the room for ever.
            text: model.sendState === "failed"
                  ? qsTr("Discard") : qsTr("Delete")
            visible: row.heldLink.length === 0
                     && (!host.isLandscape || model.kind === "undecryptable")
                     && (row.isOwn || row.canDeleteForOther)
            onClicked: host.confirmDelete(model.eventId || "",
                                          model.txnId || "",
                                          model.sendState === "failed")
        }

        MenuItem {
            text: qsTr("Pin")
            // Only where this account may write the pinned list. `!== false`: until the
            // room answers there is no entry, and a slow answer must not remove an action.
            visible: model.kind === "message" && (model.eventId || "").length > 0
                     && !host.isLandscape
                     && matrix.roomPermissions.pin !== false
            onClicked: matrix.pinMessage(model.eventId, true)
        }




        MenuItem {
            // Starting one, not only answering in one that exists: the marker opens a
            // thread that is already there.
            text: qsTr("Reply in thread")
            visible: row.heldLink.length === 0
                     && host.inThread !== true
                     && model.kind === "message"
                     && (model.eventId || "").length > 0
                     && !host.isLandscape
            onClicked: row.actions.openThread(row.attachmentItem())
        }



        MenuItem {
            // The queue gave up on this one; this puts it back in
            // line. Nothing else in the app could move it.
            text: qsTr("Send again")
            visible: row.heldLink.length === 0
                     && model.sendState === "failed" && !host.isLandscape
            onClicked: matrix.retryMessage(model.txnId || "")
        }



        MenuItem {
            text: qsTr("Save")
            visible: row.heldLink.length === 0
                     && (row.isFile || row.isImage) && !host.isLandscape
            onClicked: row.actions.saveAttachment(row.attachmentItem())
        }

        MenuItem {
            text: qsTr("Convert to text")
            visible: row.heldLink.length === 0
                     && row.canTranscribe && !host.isLandscape
                     && row.transcriptState !== "working"
                     && row.transcriptState !== "done"
            onClicked: matrix.transcripts.transcribe(row.mediaId, model.eventId, model.media)
        }



        MenuItem {
            text: qsTr("Copy")
            visible: row.heldLink.length === 0
                     && (model.body || "").length > 0
            onClicked: Clipboard.text = model.body
        }



        MenuItem {
            text: qsTr("Forward")
            // An attachment goes as the file, never as its name: hung off the body
            // alone, this forwarded a video as the sentence "clip.mp4".
            visible: row.heldLink.length === 0
                     && (row.isFile || row.isImage
                      || (model.body || "").length > 0)
                     && !host.isLandscape
            onClicked: {
                if (row.isFile || row.isImage) {
                    row.actions.forwardAttachment(row.attachmentItem())
                    return
                }
                pageStack.push(Qt.resolvedUrl("ForwardPage.qml"), {
                                   body: model.body
                               })
            }
        }



        MenuItem {
            text: qsTr("Edit")
            visible: row.heldLink.length === 0
                     && row.isOwn && model.editable === true && !host.isLandscape
                     && !host.inSlice
            onClicked: host.beginEdit(model.eventId, model.body)
        }



        MenuItem {
            text: qsTr("React")
            visible: row.heldLink.length === 0
                     && model.kind === "message"
                     && (model.eventId || "").length > 0
                     && !host.isLandscape
            onClicked: row.actions.pickReaction(model.eventId)
        }



        MenuItem {
            text: qsTr("Reply")
            visible: row.heldLink.length === 0
                     && model.kind === "message"
            onClicked: host.beginReply(model.eventId, model.senderName, model.body)
        }



        MenuItem {
            text: qsTr("More…")
            visible: row.heldLink.length === 0
                     && model.kind === "message"
                     && host.isLandscape
            onClicked: row.showActions()
        }
    }

    LinkHighlight {
        id: linkHighlight

        label: bodyLabel
        marks: row.linkMarks
    }

    // The sender's picture, for other people only. Keyed by the address, so five
    // senders in two hundred messages cost five downloads.
    Avatar {
        id: senderAvatar

        visible: row.isBubble && (!row.isOwn || host.flatLayout) && !row.continues
        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
            top: parent.top
            // Level with the name line, which sits inside the column's padding.
            topMargin: host.flatLayout ? Theme.paddingMedium : 0
        }
        size: host.avatarSize
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
                           { roomId: host.roomId, userId: model.sender })
            onPressAndHold: row.holdMenu()
        }
    }

    // A message. Own messages sit on the right in the highlight colour,
    // everyone else's on the left — the usual reading cue.
    Rectangle {
        id: bubble

        visible: row.isBubble
        anchors {
            right: row.isOwn && !host.flatLayout ? parent.right : undefined
            left: row.isOwn && !host.flatLayout ? undefined : parent.left
            rightMargin: Theme.horizontalPageMargin
            // A constant, never the avatar's width: reading a sibling's width here and
            // its height there is how this delegate earns a binding loop.
            leftMargin: host.flatLayout
                        ? Theme.horizontalPageMargin - Theme.paddingMedium
                        : Theme.horizontalPageMargin
                          + host.avatarSize + Theme.paddingSmall
            top: parent.top
        }
        // Width follows the column, which sizes itself from the texts. Deriving it
        // from the labels would close the loop - their width comes from the bubble.
        width: bubbleColumn.width + 2 * Theme.paddingMedium
        height: bubbleColumn.height + 2 * Theme.paddingMedium
        radius: Theme.paddingMedium
        // Both fills stay faint: an opaque highlight can land on any lightness under
        // an ambience. The stronger tint marks the own side; both are settable.
        color: host.flatLayout ? "transparent"
               : row.isOwn
               ? Theme.rgba(appearance.ownBubbleColor.length > 0
                            ? appearance.ownBubbleColor
                            : Theme.highlightBackgroundColor,
                            appearance.ownBubbleOpacity)
               : Theme.rgba(appearance.otherBubbleColor.length > 0
                            ? appearance.otherBubbleColor
                            : Theme.highlightBackgroundColor,
                            appearance.otherBubbleOpacity)
        // A held link is marked alone: the bubble steps back with the rest of the host.
        opacity: row.linkMarks.length > 0 ? Theme.opacityLow
                                         : (model.pending === true ? 0.5 : 1.0)

        Column {
            id: bubbleColumn

            // The widest a bubble's text may get before wrapping. Someone else's starts
            // further right and gets correspondingly less.
            property real maxTextWidth: host.flatLayout
                                        ? row.listView.width
                                          - 2 * Theme.horizontalPageMargin
                                        : (host.messageLayout === "wide"
                                           ? row.listView.width
                                             - 2 * Theme.horizontalPageMargin
                                           : row.listView.width * 0.82)
                                          - 2 * Theme.paddingMedium
                                          - (row.isOwn ? 0 : host.avatarSize
                                                            + Theme.paddingSmall)
            // Where the name starts: beside the picture when it heads the row.
            readonly property real senderInset: host.flatLayout
                                                ? host.avatarSize + Theme.paddingSmall
                                                : 0

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
            readonly property bool holdRight: row.isOwn && !host.flatLayout

            Label {
                id: senderLabel

                anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                x: bubbleColumn.senderInset
                width: Math.min(implicitWidth,
                                bubbleColumn.maxTextWidth - bubbleColumn.senderInset)
                height: host.flatLayout ? host.avatarSize : implicitHeight
                verticalAlignment: Text.AlignVCenter
                visible: (!row.isOwn || host.flatLayout) && !row.continues
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
                // Not for a thread row in the room: the SDK marks every threaded event a
                // reply to the previous one there. A thread's own view keeps only real replies.
                visible: !!model.replyTo
                         && ((model.threadRoot || "").length === 0 || host.inThread === true)
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

                        // EXIF rotation, as every other client draws it.
                        autoTransform: true

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
                            if (matrix.mediaPath(mediaKey).length > 0) {
                                source = matrix.imageSource(mediaKey)
                            } else if (!settings.autoLoadMedia) {
                                // "Load pictures: off" covers quotes.
                                return
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
                                    quoteThumb.source = matrix.imageSource(key)
                                }
                            }
                        }

                        Connections {
                            target: row.actions
                            onMediaRetry: {
                                if (quoteThumb.source == "") {
                                    quoteThumb.load()
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
                    onClicked: host.jumpToEvent(model.replyTo.eventId)
                    // The long press still belongs to the message, not to the
                    // quote — otherwise this corner of the bubble has no menu.
                    onPressAndHold: row.holdMenu()
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

                    // EXIF rotation, as every other client draws it.
                    autoTransform: true

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
                        // Measured from the file that arrived; a refused
                        // one (SVG, oversized) is still there to save.
                        return matrix.imageSource(row.mediaId)
                    }

                    /// On disk but not handed to a decoder; a tap opens the viewer.
                    /// `arrivals` re-reads it: the bridge's answers carry no signal.
                    property int arrivals: 0
                    property bool refused: arrivals >= 0 && row.hasPreview
                                           && matrix.mediaPath(row.mediaId).length > 0
                                           && !matrix.mediaShowable(row.mediaId)

                    // A request is out; the spinner belongs to it.
                    property bool requested: false

                    Component.onCompleted: fetch(false)

                    // `asked`: a tap, which overrides "load pictures: off".
                    function fetch(asked) {
                        if (!row.hasPreview || source != "" || refused) {
                            return
                        }
                        // Only when asked, where the user said so: scrolling past a picture
                        // is a request to the sender's server. The frame stays tappable.
                        if (!asked && !settings.autoLoadMedia) {
                            return
                        }
                        requested = true
                        // A sender's thumbnail is already small and, in encrypted rooms, the only one
                        // there is. A video has no other preview at all.
                        if (model.media.thumbnailSource) {
                            matrix.requestMedia(row.mediaId, model.media.thumbnailSource, false,
                                                model.media.size || 0)
                        } else {
                            matrix.requestMedia(row.mediaId, model.media.source, true,
                                                model.media.size || 0)
                        }
                    }

                    // Play affordance over the still.
                    Image {
                        anchors.centerIn: parent
                        visible: (row.isVideo || row.isAnimated)
                                 && attachment.source != ""
                        source: "image://theme/icon-l-play?" + Theme.lightPrimaryColor
                    }

                    // Decided before opening: a video that turns out to be loud
                    // should never get to play a note. Above the tap area, so it
                    // answers for itself.
                    IconButton {
                        // Left, as on the player: the same control belongs in
                        // the same corner in both places.
                        anchors {
                            left: parent.left
                            bottom: parent.bottom
                            margins: Theme.paddingSmall
                        }
                        z: 1
                        visible: row.isVideo && attachment.source != ""
                        opacity: 0.7
                        icon.source: row.actions.mutedVideos[row.mediaId] === true
                                     ? "image://theme/icon-m-speaker-mute"
                                     : "image://theme/icon-m-speaker"
                        onClicked: {
                            // Reassigned, not mutated in place, as with
                            // `failedMedia`: a binding is told nothing otherwise.
                            var chosen = row.actions.mutedVideos
                            chosen[row.mediaId] = chosen[row.mediaId] !== true
                            row.actions.mutedVideos = chosen
                            row.actions.mutedVideosChanged()
                        }
                    }

                    Connections {
                        target: matrix
                        onMediaReady: {
                            if (key === row.mediaId) {
                                attachment.source = matrix.imageSource(key)
                                attachment.arrivals += 1
                            }
                        }
                    }

                    Connections {
                        target: row.actions
                        onMediaRetry: attachment.fetch(false)
                    }

                    BusyIndicator {
                        anchors.centerIn: parent
                        size: BusyIndicatorSize.Medium
                        // Not while waiting for something that is not coming: a refused or failed
                        // download left this turning for the life of the host.
                        running: attachment.visible && attachment.source == ""
                                 && attachment.requested && !attachment.refused
                                 && !row.actions.failedMedia[row.mediaId]
                    }

                    // Failed or not yet asked for: the frame says what a tap does.
                    readonly property bool waitsForTap: visible && source == ""
                                                         && (!!row.actions.failedMedia[row.mediaId]
                                                             || !requested || refused)

                    Image {
                        id: retryIcon

                        anchors {
                            horizontalCenter: parent.horizontalCenter
                            verticalCenter: parent.verticalCenter
                            verticalCenterOffset: -retryLabel.height / 2
                        }
                        visible: attachment.waitsForTap
                        source: row.actions.failedMedia[row.mediaId]
                                ? "image://theme/icon-m-refresh?" + Theme.secondaryColor
                                : "image://theme/icon-m-image?" + Theme.secondaryColor
                    }

                    Label {
                        id: retryLabel

                        anchors {
                            top: retryIcon.bottom
                            topMargin: Theme.paddingSmall
                            horizontalCenter: parent.horizontalCenter
                        }
                        width: parent.width - 2 * Theme.paddingMedium
                        visible: attachment.waitsForTap
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.secondaryColor
                        text: attachment.refused
                              ? qsTr("Not shown on this device. Tap to open.")
                              : row.actions.failedMedia[row.mediaId]
                                ? qsTr("Could not load. Tap to try again.")
                                : qsTr("Tap to load")
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: row.hasPreview
                        onClicked: {
                            if (attachment.source == "" && !attachment.refused) {
                                row.actions.forgetFailedMedia(row.mediaId)
                                attachment.fetch(true)
                                return
                            }
                            if (row.isVideo) {
                                row.actions.openVideo(row.mediaId, model.media)
                            } else {
                                row.actions.openImage(row.mediaId, model.media)
                            }
                        }
                        // The delegate's own menu still has to be reachable.
                        onPressAndHold: row.holdMenu()
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
                    icon.source: row.actions.playingId === row.mediaId && row.player.playbackState === Audio.PlayingState
                                 ? "image://theme/icon-m-pause"
                                 : "image://theme/icon-m-play"
                    onClicked: row.actions.toggleAudio(row.mediaId, model.media)
                }

                Label {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.min(implicitWidth,
                                    bubbleColumn.maxTextWidth - Theme.itemSizeSmall)
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryColor
                    truncationMode: TruncationMode.Fade
                    text: row.actions.playingId === row.mediaId && row.player.duration > 0
                          ? Format.formatDuration(Math.round(row.player.position / 1000),
                                                  Formatter.DurationShort)
                            + " / "
                            + Format.formatDuration(Math.round(row.player.duration / 1000),
                                                    Formatter.DurationShort)
                          : (model.body || qsTr("Voice message"))
                }
            }

            // A voice message converted on request: text a machine heard, from a
            // stranger. Plain, capped in src/voicetranscripts.cpp, never linked.
            Label {
                id: transcriptLabel

                anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                // Measured by its twin, as the body is: a wrapped label's own implicit
                // width follows its laid-out width, and reading that back loops.
                width: Math.min(transcriptMeasure.implicitWidth,
                                bubbleColumn.maxTextWidth)
                visible: row.isAudio && row.transcriptState.length > 0

                Label {
                    id: transcriptMeasure

                    visible: false
                    textFormat: Text.PlainText
                    font.pixelSize: transcriptLabel.font.pixelSize
                    text: transcriptLabel.text
                }
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: {
                    if (row.transcriptState !== "done") {
                        return Theme.secondaryColor
                    }
                    var own = row.isOwn ? appearance.ownTextColor
                                        : appearance.otherTextColor
                    return own.length > 0 ? own : Theme.primaryColor
                }
                text: {
                    if (matrix.transcripts.revision < 0) {
                        return ""
                    }
                    switch (row.transcriptState) {
                    case "working": return qsTr("Converting to text…")
                    case "failed": return matrix.transcripts.failure(row.rowEventId)
                    case "done": return matrix.transcripts.text(row.rowEventId)
                    default: return ""
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    onPressAndHold: row.holdMenu()
                }
            }

            Label {
                anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                width: Math.min(implicitWidth, bubbleColumn.maxTextWidth)
                visible: row.isAudio && row.transcriptState === "done"
                truncationMode: TruncationMode.Fade
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                textFormat: Text.PlainText
                text: qsTr("Recognised automatically, may contain mistakes")
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
                visible: !row.isPoll && !row.isLocation
                         && ((!row.hasPreview && !row.isAudio)
                             || row.hasCaption)

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
                // The unfolded height, at the label's own width. Short text cannot fill a host.
                Label {
                    id: bodyFull

                    visible: false
                    width: bodyLabel.width
                    wrapMode: Text.Wrap
                    textFormat: bodyLabel.textFormat
                    font.pixelSize: bodyLabel.font.pixelSize
                    text: bodyLabel.text.length > 40 ? bodyLabel.text : ""
                }
                maximumLineCount: row.longBody && !row.expanded ? 3 : 1000000
                elide: row.longBody && !row.expanded ? Text.ElideRight : Text.ElideNone
                // Plain text on purpose: a body is untrusted and AutoText renders anything
                // HTML-shaped, `<img>` included. StyledText only after MessageBody escaped it.
                textFormat: row.richBody.length > 0
                            ? Text.StyledText : Text.PlainText
                linkColor: Theme.highlightColor
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
                            row.actions.openVideo(row.mediaId, model.media)
                            return
                        }
                        row.actions.openImage(row.mediaId, model.media)
                    }
                    onPressAndHold: row.holdMenu()
                }

                // The thread asks the same function.
                text: MessageBody.rowText({
                    "kind": model.kind,
                    "utdCause": model.utdCause || "",
                    "body": model.body || "",
                    "caption": row.captionText,
                    "media": model.kind === "message" && !!model.media,
                    "image": row.isImage,
                    "drawn": row.hasPreview || row.isAudio,
                    "rich": row.richBody
                })
            }

            Label {
                anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                visible: bodyLabel.visible && row.longBody
                width: Math.min(implicitWidth, bubbleColumn.maxTextWidth)
                font.pixelSize: Theme.fontSizeExtraSmall
                color: foldArea.pressed ? Theme.secondaryHighlightColor
                                        : Theme.highlightColor
                textFormat: Text.PlainText
                text: row.expanded ? qsTr("Show less") : qsTr("Show more")

                MouseArea {
                    id: foldArea
                    anchors {
                        fill: parent
                        margins: -Theme.paddingSmall
                    }
                    onClicked: host.toggleExpanded(model.id)
                    onPressAndHold: row.holdMenu()
                }
            }

            // The poll. Loaded only for a poll row - two hundred rows must
            // not carry its repeater - and its width handed in, never
            // measured back out of it.
            Loader {
                anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                active: row.isPoll
                visible: active
                sourceComponent: PollBlock {
                    availableWidth: bubbleColumn.maxTextWidth
                    poll: model.poll
                    eventId: row.rowEventId
                    own: row.isOwn
                    // Not before the server has it: an echo has no id to vote on.
                    pending: model.pending === true
                    threadRoot: row.actions ? row.actions.threadRoot : ""
                }
            }

            // A location or a live share. Width handed in, as for the poll.
            Loader {
                anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                active: row.isLocation
                visible: active
                sourceComponent: LocationBlock {
                    availableWidth: bubbleColumn.maxTextWidth
                    location: model.location
                    own: row.isOwn
                    avatarSource: model.senderAvatar || ""
                    avatarName: model.senderName || model.sender || ""
                    roomId: host.roomId
                    encrypted: host.encrypted || !host.encryptionKnown
                    onActivated: host.followLink(link, map)
                }
            }

            // The linked page, as the homeserver describes it. Loaded only
            // where a preview may be asked for, shown once there is one.
            Loader {
                id: previewLoader

                anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                active: row.previewUrl.length > 0
                visible: active && !!item && item.available
                sourceComponent: LinkPreviewCard {
                    availableWidth: bubbleColumn.maxTextWidth
                    url: row.previewUrl
                    // The same switch the link in the text obeys. Without
                    // it "links stay plain text" left the card openable.
                    tappable: settings.clickableLinks
                    onActivated: row.holdLink(link)
                    onHeld: row.holdMenu()
                }
            }

            // What the SDK will not vouch for: red is a message that is not what it
            // claims to be, grey one it cannot check. Both silent otherwise.
            Label {
                anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                width: Math.min(implicitWidth, bubbleColumn.maxTextWidth)
                visible: row.isBubble && !!model.shield
                         && row.actions.shieldEventId === model.eventId
                font.pixelSize: Theme.fontSizeExtraSmall
                color: model.shield && model.shield.level === "red"
                       ? row.actions.shieldRedColor : row.actions.shieldGreyColor
                textFormat: Text.PlainText
                text: row.actions.shieldText(model.shield)
            }

            // The way from one sentence to all of them, shown with the sentence: a mark
            // that is never tapped needs no glossary either.
            Label {
                anchors.right: bubbleColumn.holdRight ? parent.right : undefined
                visible: row.isBubble && !!model.shield
                         && row.actions.shieldEventId === model.eventId
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

                visible: row.isBubble && host.inThread !== true
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
                    onClicked: row.actions.openThread(row.attachmentItem())
                    onPressAndHold: row.holdMenu()
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
                                    host.showNotice(qsTr("This reaction hides text and was not sent"))
                                    return
                                }
                                row.actions.react(row.rowEventId, sendKey)
                            }
                            // Held down, the chip says who set it. It takes the long press for the width
                            // of a chip; the rest of the bubble keeps the message's menu.
                            onPressAndHold: row.actions.showReactors(
                                                row.rowEventId,
                                                modelData.sendKey || modelData.key,
                                                modelData.key)

                            Rectangle {
                                anchors.fill: parent
                                radius: Theme.paddingSmall
                                // Ours is the one worth telling
                                // apart; the rest carry the bubble.
                                // Own reaction the queue gave up; a tap withdraws it.
                                color: modelData.failed
                                       ? Theme.rgba(Theme.errorColor, 0.35)
                                       : modelData.mine
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
                                senderLabel.visible
                                ? bubbleColumn.senderInset + senderLabel.width : 0,
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
                                   ? row.actions.shieldRedColor
                                   : row.actions.shieldGreyColor
                        }

                        MouseArea {
                            // Larger than the glyph: a triangle at
                            // tiny size is not a touch target.
                            anchors.centerIn: parent
                            width: Theme.itemSizeExtraSmall
                            height: Theme.itemSizeExtraSmall
                            onClicked: row.actions.showShield(model.eventId)
                        }
                    }

                    Label {
                        id: metaLabel

                        anchors.verticalCenter: parent.verticalCenter
                        font.pixelSize: Theme.fontSizeTiny
                        // A message that did not get out is the one thing in this line worth a colour.
                        color: model.sendState === "failed"
                               || model.editState === "failed"
                               || model.redactionState === "failed"
                               ? Theme.errorColor : Theme.secondaryColor
                        // Only messages carry a timestamp; the shared delegate instantiates this
                        // label for every row regardless.
                        text: row.isBubble
                              ? (matrix.pinnedEventIds.indexOf(model.eventId) >= 0 ? "📌 " : "")
                                + (model.sendState === "failed"
                                   ? qsTr("not sent") + " · " : "")
                                // Edit or deletion not sent; the banner has the way out.
                                + (model.redactionState === "failed"
                                   ? qsTr("deletion not sent") + " · " : "")
                                + (model.editState === "failed"
                                   ? qsTr("edit not sent") + " · " : "")
                                + (model.edited === true ? qsTr("edited") + " · " : "")
                                + (row.mediaSizeText.length > 0
                                   ? row.mediaSizeText + " · " : "")
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
                            onClicked: row.actions.showReaders(model.eventId)
                            onPressAndHold: row.actions.showReaders(model.eventId)
                        }
                    }
                }
            }

            // Who they are, as one line: a name per row would push the conversation off
            // the screen for an aside.
            Label {
                id: readersLabel

                anchors.right: bubbleColumn.holdRight ? parent.right : undefined

                visible: row.actions.namesEventId === model.eventId
                         && row.actions.namesText.length > 0
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
                text: readersLabel.visible ? row.actions.namesText : ""

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
        // Two device pixels in the full colour: it is the one thing that says
        // where the new messages start, and at one faint pixel it was missed.
        height: Math.max(2, Math.round(2 * Theme.pixelRatio))
        color: Theme.highlightColor
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

    // Who acted. Fixed size and fixed left edge, so the line beside it
    // never measures back.
    Avatar {
        id: systemAvatar

        visible: row.isSystem
        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
            verticalCenter: systemLabel.verticalCenter
        }
        size: host.systemAvatarSize
        source: model.senderAvatar || ""
        name: model.senderName || model.sender || ""

        // The same tap as on a message avatar. No press-and-hold: a
        // system line has no menu. The margin stops at the text, or it
        // grows a target over words it does not open.
        MouseArea {
            anchors {
                fill: parent
                margins: -Theme.paddingMedium
            }
            onClicked: pageStack.push(
                           Qt.resolvedUrl("MemberProfilePage.qml"),
                           { roomId: host.roomId, userId: model.sender })
        }
    }

    Label {
        id: systemTime

        visible: row.isSystem
        anchors {
            right: parent.right
            rightMargin: Theme.horizontalPageMargin
            verticalCenter: systemLabel.verticalCenter
        }
        font.pixelSize: Theme.fontSizeTiny
        color: Theme.secondaryColor
        textFormat: Text.PlainText
        text: row.isSystem
              ? Format.formatDate(new Date(model.timestamp), Formatter.TimeValue)
              : ""
    }

    Label {
        id: systemLabel

        visible: row.isSystem
        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
                        + host.systemAvatarSize + Theme.paddingMedium
            top: parent.top
            topMargin: Theme.paddingMedium
        }
        // A system line carries two names and a reason; it wraps rather
        // than fading away, and stops after three lines.
        // Between picture and time: both size themselves from their own
        // content, so reading the time's width closes no loop.
        width: row.listView.width - 2 * Theme.horizontalPageMargin
               - host.systemAvatarSize - 2 * Theme.paddingMedium
               - systemTime.width
        wrapMode: Text.Wrap
        maximumLineCount: 3
        elide: Text.ElideRight
        // Names and reasons are text a stranger wrote: AutoText would
        // hand a body with a tag to the rich-text renderer.
        textFormat: Text.PlainText
        font.pixelSize: Theme.fontSizeExtraSmall
        font.italic: true
        color: Theme.secondaryColor
        text: row.isSystem ? row.systemText() : ""
    }
}
