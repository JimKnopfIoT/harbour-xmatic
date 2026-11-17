import QtQuick 2.5
import Sailfish.Silica 1.0
import QtMultimedia 5.6
import "SecurityStatus.js" as SecurityStatus
import "MatrixLinks.js" as MatrixLinks

// What a timeline row asks of its page beyond the composer: media opened, saved
// and forwarded, the voice player, reactions, the names and shield lines.
Item {
    id: actions

    /// The page the actions belong to; notices appear on it.
    property Item page
    /// The page's one voice player.
    property var player
    /// Set on a thread's page: reactions and names go to the thread's own view.
    property string threadRoot: ""
    /// Prefixed to row ids for media keys; a thread's rows are numbered apart.
    property string mediaPrefix: ""
    /// The model the rows come from; it learns which system lines are hidden.
    property QtObject timelineModel: null

    // Which room events get a line; the switches are on the behaviour page.
    function systemShown(kind) {
        switch (kind) {
        case "member.joined":   return true
        case "member.left":     return behaviour.showLeaves
        case "profile.name":    return behaviour.showDisplayNameChanges
        case "profile.avatar":  return behaviour.showAvatarChanges
        case "profile.both":    return behaviour.showDisplayNameChanges || behaviour.showAvatarChanges
        case "profile":
        case "member.none":     return false
        default:                return true
        }
    }

    // The tokens above that can be off, as the model needs them to join a sender's run.
    readonly property var hiddenSystems: {
        var tokens = ["member.left", "profile.name", "profile.avatar", "profile.both",
                      "profile", "member.none"]
        return tokens.filter(function(token) { return !actions.systemShown(token) })
    }

    Binding {
        target: actions.timelineModel
        property: "hiddenSystems"
        value: actions.hiddenSystems
        when: actions.timelineModel !== null
    }

    // The thread a row belongs to, or the one it starts. Menu, marker and the
    // actions page all go through here.
    function threadRootOf(item) {
        return item.threadRoot && item.threadRoot.length > 0 ? item.threadRoot : (item.eventId || "")
    }

    function openThread(item) {
        var root = actions.threadRootOf(item)
        if (root.length === 0) {
            return
        }
        pageStack.push(Qt.resolvedUrl("ThreadPage.qml"), {
                           roomId: actions.page.roomId,
                           roomName: actions.page.roomName,
                           rootEventId: root,
                           encrypted: actions.page.encrypted,
                           encryptionKnown: actions.page.encryptionKnown
                       })
    }

    // One voice message at a time across pages: the room's player stops when the thread's starts.
    function play() {
        pageStack.find(function(other) {
            var theirs = other.timelineActions
            if (theirs && theirs !== actions && theirs.player) {
                theirs.player.pause()
            }
            return false
        })
        actions.player.play()
    }

    // A tapped link. A Matrix address is answered inside the app; anything else is a
    // web address. `map`: a location card's cached map, shown under the address.
    function followLink(link, map) {
        var target = MatrixLinks.decide(link)
        if (target.kind === "none") {
            return
        }
        if (target.kind === "web") {
            // The address before it is opened: what a link says and where it
            // goes are two strings a stranger writes.
            var properties = {
                question: qsTranslate("RoomPage", "Open this address?"),
                subject: link,
                acceptLabel: qsTranslate("RoomPage", "Open"),
                plainCancel: true
            }
            if (map && map.available === true) {
                properties.extraSource = Qt.resolvedUrl("LocationMap.qml")
                properties.extraProperties = { mapData: map }
            }
            var dialog = pageStack.push(Qt.resolvedUrl("ConfirmDialog.qml"), properties)
            dialog.accepted.connect(function() { Qt.openUrlExternally(link) })
            return
        }
        if (target.kind === "user") {
            // Not a silent direct chat: the dialog names the address and the
            // user accepts it.
            pageStack.push(Qt.resolvedUrl("NewChatDialog.qml"), { prefill: target.id })
            return
        }
        actions.pendingAddress = target.id
        matrix.resolveRoom(target.id)
    }

    // The address a tapped link asked about, until the core has answered.
    property string pendingAddress: ""

    Connections {
        target: matrix

        onRoomResolved: {
            // Only the answer to this page's request, not another page's link.
            if (local || actions.pendingAddress.length === 0
                    || address !== actions.pendingAddress.trim()) {
                return
            }
            actions.pendingAddress = ""
            if (joined) {
                pageStack.push(Qt.resolvedUrl("RoomPage.qml"), { roomId: roomId, roomName: "" })
                return
            }
            // Never from the link itself: joining is the user's decision, and
            // the dialog is where it is made.
            pageStack.push(Qt.resolvedUrl("JoinRoomDialog.qml"), { prefill: address })
        }

        onRoomResolveFailed: {
            if (!local && address === actions.pendingAddress.trim()) {
                actions.pendingAddress = ""
            }
        }
    }

    function react(eventId, key) {
        matrix.toggleReaction(eventId, key, actions.threadRoot)
    }

    // Media keys whose fetch failed - refused, dropped, given up on. Without them
    // the row cannot tell "still loading" from "never coming".
    property var failedMedia: ({})
    // Delegates ask again for what they lack.
    signal mediaRetry()

    function forgetFailedMedia(key) {
        var marked = actions.failedMedia
        delete marked[key]
        actions.failedMedia = marked
    }

    // A save that is waiting for its download to finish.
    property string pendingSaveKey: ""
    /// Forwarding an attachment needs the file on this device first - the target
    /// room encrypts under its own keys, and a thumbnail is not the file.
    property string pendingForwardKey: ""
    property string pendingForwardMime: ""

    // The audio item currently loaded into the player, and one waiting for its
    // download.
    property string playingId: ""
    /// Videos chosen to open silent, by row id. Per video and per visit: it
    /// answers "not this one, not now", not "never again".
    property var mutedVideos: ({})
    property string pendingPlayKey: ""
    property bool pendingSaveIsImage: false
    property string pendingSaveName: ""

    Connections {
        target: matrix

        onMediaFailed: {
            var marked = actions.failedMedia
            marked[key] = true
            // Reassigned, not mutated in place: a JavaScript object changed
            // through its own reference tells no binding anything.
            actions.failedMedia = marked
        }

        // Back online: a failed picture gets a second attempt.
        onSyncStateChanged: {
            if (matrix.syncState === "running" && Object.keys(actions.failedMedia).length > 0) {
                actions.failedMedia = ({})
                actions.mediaRetry()
            }
        }
    }

    Connections {
        target: matrix
        onMediaReady: {
            if (key === actions.pendingPlayKey) {
                actions.pendingPlayKey = ""
                actions.player.source = "file://" + path
                actions.play()
                return
            }
            if (key === actions.pendingForwardKey) {
                actions.pendingForwardKey = ""
                pageStack.push(Qt.resolvedUrl("ForwardPage.qml"), {
                                   path: "file://" + path,
                                   mimeType: actions.pendingForwardMime
                               })
                return
            }
            if (key !== actions.pendingSaveKey) {
                return
            }
            actions.pendingSaveKey = ""
            actions.storeTo(path, actions.pendingSaveName, actions.pendingSaveIsImage)
        }
    }

    function toggleAudio(itemId, media) {
        if (actions.playingId === itemId) {
            if (actions.player.playbackState === Audio.PlayingState) {
                actions.player.pause()
            } else {
                actions.play()
            }
            return
        }

        var key = itemId + "/full"
        var known = matrix.mediaPath(key)
        actions.playingId = itemId
        if (known.length > 0) {
            actions.player.source = "file://" + known
            actions.play()
            return
        }
        actions.pendingPlayKey = key
        matrix.requestMedia(key, media.source, false, media.size || 0)
    }

    function openVideo(itemId, media) {
        var silent = actions.mutedVideos[itemId] === true
        var key = itemId + "/full"
        var known = matrix.mediaPath(key)
        matrix.requestMedia(key, media.source, false, media.size || 0)
        pageStack.push(Qt.resolvedUrl("VideoPage.qml"), {
                           mediaKey: key,
                           source: known.length > 0 ? "file://" + known : "",
                           fileName: media.filename || "",
                           mimeType: media.mimetype || "",
                           declaredSize: media.size || 0,
                           startMuted: silent
                       })
    }

    // Forwarding re-sends rather than references: the target room encrypts under its
    // own keys. `item.id` is the row's media key (`MessageRow.attachmentItem`).
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
        actions.pendingForwardKey = key
        actions.pendingForwardMime = mime
        matrix.requestMedia(key, item.media.source, false, item.media.size || 0)
    }

    function saveAttachment(item) {
        var key = item.id + "/full"
        var known = matrix.mediaPath(key)
        if (known.length > 0) {
            actions.storeFile(known, item)
            return
        }
        actions.pendingSaveKey = key
        actions.pendingSaveIsImage = item.msgtype === "m.image"
        actions.pendingSaveName = item.media ? (item.media.filename || "") : ""
        matrix.requestMedia(key, item.media.source, false, item.media.size || 0)
    }

    // Choosing a reaction for a message. The dialog hands the character back;
    // sending it is a toggle, so choosing the one already given takes it back.
    function pickReaction(eventId) {
        var dialog = pageStack.push(Qt.resolvedUrl("ReactionDialog.qml"),
                                    { eventId: eventId })
        dialog.accepted.connect(function() {
            if (dialog.key.length > 0) {
                actions.react(eventId, dialog.key)
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
        actions.namesEventId = ""
        actions.namesKey = ""
        actions.namesPrefix = ""
        actions.namesText = ""
    }

    /// Toggle: a second tap on the same mark puts the sentence away again.
    function showShield(eventId) {
        actions.shieldEventId = actions.shieldEventId === eventId ? "" : eventId
    }

    function showReaders(eventId) {
        if (actions.namesEventId === eventId && actions.namesKey.length === 0) {
            actions.clearNames()
            return
        }
        actions.clearNames()
        actions.namesEventId = eventId
        matrix.loadReaders(eventId, actions.threadRoot)
    }

    /// Press and hold on a reaction: who set it. The key that goes to the core is
    /// the one the row sends with, not the one it draws.
    function showReactors(eventId, sendKey, shownKey) {
        if (actions.namesEventId === eventId && actions.namesKey === sendKey) {
            actions.clearNames()
            return
        }
        actions.clearNames()
        actions.namesEventId = eventId
        actions.namesKey = sendKey
        actions.namesPrefix = shownKey + " "
        matrix.loadReactors(eventId, sendKey, actions.threadRoot)
    }

    Connections {
        target: matrix
        onReadersReady: {
            if (eventId !== actions.namesEventId || actions.namesKey.length > 0) {
                return
            }
            var names = []
            for (var i = 0; i < readers.length; i++) {
                names.push(readers[i].name)
            }
            actions.namesText = names.join(", ")
        }

        onReactorsReady: {
            if (eventId !== actions.namesEventId || key !== actions.namesKey) {
                return
            }
            var who = []
            for (var i = 0; i < reactors.length; i++) {
                who.push(reactors[i].name)
            }
            // Nobody to name: fold the line away rather than leave the reaction standing
            // alone, which reads as a line that failed to load.
            if (who.length === 0) {
                actions.clearNames()
                return
            }
            actions.namesText = actions.namesPrefix + who.join(", ")
        }
    }

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

    function storeFile(path, item) {
        var name = item.media ? item.media.filename : ""
        actions.storeTo(path, name, item.msgtype === "m.image")
    }

    function storeTo(path, name, isImage) {
        if (isImage) {
            actions.page.showNotice(matrix.saveToPictures(path, name).length > 0
                            ? qsTr("Saved to gallery") : qsTr("Could not save"))
        } else {
            actions.page.showNotice(matrix.saveToDownloads(path, name).length > 0
                            ? qsTr("Saved to Downloads") : qsTr("Could not save"))
        }
    }

    function openImage(itemId, media) {
        // A separate cache key: the timeline holds the thumbnail under the
        // plain item id, and both are worth keeping.
        var key = itemId + "/full"
        matrix.requestMedia(key, media.source, false, media.size || 0)
        pageStack.push(Qt.resolvedUrl("ImageViewPage.qml"), {
                           mediaKey: key,
                           fileName: media.filename || "",
                           mimeType: media.mimetype || "image/jpeg"
                       })
    }
}
