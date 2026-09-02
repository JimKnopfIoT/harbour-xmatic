import QtQuick 2.0
import Sailfish.Silica 1.0
import QtMultimedia 5.6
import Nemo.Notifications 1.0
import Nemo.KeepAlive 1.2
import Sailfish.Share 1.0

import "pages"
import "pages/SecurityStatus.js" as SecurityStatus

ApplicationWindow {
    id: app

    // Which root page is up, so it is only exchanged when it really changes.
    // Assigned once, never bound: a binding would beat `showPageFor` to the change.
    property string shownRoot: ""

    // Whether the security page has already been offered in this run. Once per
    // app start — it leads, it does not nag.
    property bool securityShown: false

    // The room the standing notification is about, or "" if none stands.
    property string notifiedRoomId: ""

    // Decided here rather than corrected a moment later: the login page must not
    // flash up on a device that has to be told what to install.
    initialPage: Qt.resolvedUrl(matrix.storageBlocked
                                ? "pages/StorageBlockedPage.qml"
                                : "pages/LoginPage.qml")
    cover: Qt.resolvedUrl("cover/CoverPage.qml")

    // Silica's default is AutoText, which every Label inherits - a display name of
    // `<img src=http://…>` fetched a remote picture on sight.
    _defaultLabelFormat: Text.PlainText
    // Never assign `defaultAllowedOrientations`: it is read-only in Silica and
    // assigning it makes the whole window fail to load, white screen.
    allowedOrientations: defaultAllowedOrientations

    // The root a `replaceAbove` is still owed. Silica drops a push issued during a
    // transition, and the login page then never appears at all.
    property string pendingRoot: ""

    // What another app shared, until there is somewhere to put it. A share can
    // arrive before the session is restored, and a push mid-transition is dropped.
    property var pendingShare: null

    function deliverShare() {
        if (!pendingShare || matrix.sessionState !== "signed-in" || pageStack.busy) {
            return
        }
        var share = pendingShare
        pendingShare = null
        pageStack.push(Qt.resolvedUrl("pages/ShareToRoomPage.qml"), share)
    }

    function rootFor(state) {
        // Before anything else and before any login: nothing on the device and no key
        // to be had. Signing in here would create the plaintext store.
        if (matrix.storageBlocked) {
            return Qt.resolvedUrl("pages/StorageBlockedPage.qml")
        }
        // An encrypted session whose key is not at hand is not "no session": its page
        // retries, and never leads into a login that would clear the store.
        if (state === "locked") {
            return Qt.resolvedUrl("pages/SessionLockedPage.qml")
        }
        if (state !== "signed-in") {
            return Qt.resolvedUrl("pages/LoginPage.qml")
        }
        // The home page is the user's choice; the other one is reachable from
        // it with a sideways swipe (each home page attaches its sibling).
        return settings.startPage === "spaces" ? Qt.resolvedUrl("pages/SpacesPage.qml")
                                             : Qt.resolvedUrl("pages/RoomListPage.qml")
    }

    // Only a home page carries `isHome`; the login and locked pages have no
    // such property.
    function propsFor(root) {
        return root === Qt.resolvedUrl("pages/LoginPage.qml")
                || root === Qt.resolvedUrl("pages/SessionLockedPage.qml")
                || root === Qt.resolvedUrl("pages/StorageBlockedPage.qml") ? {} : { isHome: true }
    }

    function showPageFor(state) {
        var root = rootFor(state)
        if (root === shownRoot) {
            return
        }
        shownRoot = root
        if (pageStack.busy) {
            pendingRoot = root
        } else {
            pageStack.replaceAbove(null, root, propsFor(root))
        }
    }

    // Offers the security page once per run, and never over an unanswered state:
    // interrupting somebody over "unknown" is a claim the app cannot back.
    function maybeShowSecurity() {
        if (securityShown || matrix.storageBlocked) {
            return
        }
        if (matrix.sessionState !== "signed-in") {
            return
        }
        if (!SecurityStatus.known(matrix) || !SecurityStatus.needsAttention(matrix)) {
            return
        }
        if (pageStack.busy) {
            // Retried from onBusyChanged; a push during a transition is
            // dropped silently and the page would never appear.
            return
        }
        securityShown = true
        pageStack.push(Qt.resolvedUrl("pages/SecurityStatusPage.qml"))
    }

    Connections {
        target: pageStack
        // One handler, both jobs: QML refuses a type that binds the same signal twice
        // and the page then fails to load.
        onBusyChanged: {
            if (!pageStack.busy && app.pendingRoot !== "") {
                var root = app.pendingRoot
                app.pendingRoot = ""
                pageStack.replaceAbove(null, root, app.propsFor(root))
            }
            if (!pageStack.busy) {
                app.deliverShare()
                app.maybeShowSecurity()
            }
        }
    }

    // One line for a notification body from the core's preview fields.
    function previewLine(kind, text) {
        switch (kind) {
        case "text": return text
        case "emote": return "* " + text
        case "image": return qsTr("Picture")
        case "video": return qsTr("Video")
        case "audio": return qsTr("Voice message")
        case "file": return qsTr("File")
        case "location": return qsTr("Location")
        case "encrypted": return qsTr("Encrypted message")
        default: return ""
        }
    }

    // Not while the gate is up: a restore on an empty device answers "none" and
    // would take the app to the login page.
    Component.onCompleted: {
        // An assignment, not a binding - see the note on the property.
        shownRoot = rootFor(matrix.sessionState)
        if (!matrix.storageBlocked) {
            matrix.restoreSession()
        }
    }

    // The notification tap and the launcher's hand-over. Neither carries an
    // argument: which room is meant is this app's knowledge, not the caller's.
    Connections {
        target: activation

        onRaiseRequested: app.activate()

        onNotifiedRoomRequested: {
            app.activate()
            app.openNotifiedRoom()
        }
    }

    // Opens the room the standing notification is about, once - a repeated call
    // cannot walk the user through rooms. Not while the session is restoring.
    function openNotifiedRoom() {
        var roomId = app.notifiedRoomId
        if (roomId.length === 0 || matrix.sessionState !== "signed-in") {
            return
        }
        app.notifiedRoomId = ""
        notification.close()
        var current = pageStack.currentPage
        // A call takes precedence over everything; the room can wait.
        if (current && current.objectName === "callPage") {
            return
        }
        // Already there: raising the window was the whole job, and pushing a
        // second copy of the room would stack it on itself.
        if (current && current.objectName === "roomPage" && current.roomId === roomId) {
            return
        }
        // Coming from another room, the new one takes its place: swiping back belongs
        // in the chat list, not in the room the notification pulled the user out of.
        if (current && current.objectName === "roomPage") {
            pageStack.replace(Qt.resolvedUrl("pages/RoomPage.qml"),
                              { roomId: roomId, roomName: "" })
            return
        }
        pageStack.push(Qt.resolvedUrl("pages/RoomPage.qml"),
                       { roomId: roomId, roomName: "" })
    }

    Notification {
        id: notification

        appName: "xmatic"
        // Without a category a notification is silent - tone, vibration and LED all
        // hang off it. This one plays the IM tone and turns the display on.
        category: "x-nemo.messaging.im"
        // The category brings its own icon, so the app's has to be named or the
        // banner would not say who is talking.
        appIcon: "/usr/share/icons/hicolor/86x86/apps/harbour-xmatic.png"
        isTransient: false
        // Tapping opens the room this banner is about. The method takes no argument,
        // so nothing about the room stays in the system's notification store.
        remoteActions: [{
            "name": "default",
            "service": "org.xmatic.xmatic",
            "path": "/org/xmatic/xmatic",
            "iface": "org.xmatic.xmatic",
            "method": "openNotified"
        }]
    }

    // Incoming shares from other apps. The dialog finds xmatic through the
    // X-Share Method block and calls this object over D-Bus.
    ShareProvider {
        method: "room"
        // The app owns its D-Bus name itself: without this the object is registered
        // and the name is not, and the dialog starts a second copy.
        registerName: true
        capabilities: ["text/x-url", "text/plain",
                       "image/*", "video/*", "audio/*", "application/*"]

        onTriggered: {
            if (resources.length === 0) {
                return
            }

            // Only the first item: the desktop file does not claim to support
            // multiple files, so the dialog never sends more than one.
            var resource = resources[0]
            var share = { "body": "", "path": "", "mimeType": "" }
            if (resource.type === ShareResource.FilePathType) {
                // Whatever asked for the share names the file, and a path inside this app's
                // own directories is refused - token and crypto store live there.
                share.path = matrix.shareableFile(resource.filePath)
                             ? resource.filePath : ""
                share.mimeType = matrix.mimeTypeForPath(resource.filePath)
            } else {
                share.body = resource.data
            }

            // Kind only, never the content: this tells "nothing arrived" apart
            // from "it arrived and the app did nothing with it".
            console.log("xmatic: share received,",
                        share.path.length > 0 ? share.mimeType : "text")

            app.pendingShare = share
            // The share may have started the app, or found it in the
            // background; either way the room list has to be on screen.
            app.activate()
            app.deliverShare()
        }
    }

    // Sailfish suspends idle devices: without a scheduled wake-up the sync
    // connection drops while the screen is off.
    BackgroundJob {
        id: backgroundSync

        frequency: BackgroundJob.FiveMinutes
        onTriggered: catchUp.restart()
    }

    // Being awake is all the core needs; the sync service catches up on its
    // own. This just keeps the device from suspending again immediately.
    Timer {
        id: catchUp

        interval: 8000
        onTriggered: backgroundSync.finished()
    }

    Connections {
        target: Qt.application
        onStateChanged: {
            if (Qt.application.state === Qt.ApplicationActive) {
                backgroundSync.enabled = false
                notification.close()
                app.notifiedRoomId = ""
            } else {
                backgroundSync.enabled = matrix.sessionState === "signed-in"
            }
        }
    }

    // A ringing call is not a message and the system has no category for it, so
    // the app rings itself - four tones and stop.
    Audio {
        id: ringer

        source: "file:///usr/share/sounds/jolla-ringtones/stereo/jolla-ringtone.ogg"
        loops: Audio.Infinite
        volume: 0.8
    }

    Notification {
        id: callNotification

        appName: "xmatic"
        category: "x-nemo.messaging.im"
        appIcon: "/usr/share/icons/hicolor/86x86/apps/harbour-xmatic.png"
        // Stays in the event feed: the banner is gone in a moment, and a
        // missed call has to leave a trace.
        isTransient: false
        urgency: Notification.Critical
        remoteActions: [{
            "name": "default",
            "service": "org.xmatic.xmatic",
            "path": "/org/xmatic/xmatic",
            "iface": "org.xmatic.xmatic",
            "method": "activate"
        }]
    }

    Connections {
        target: matrix.calls

        // A ringing phone has to be answerable from wherever the user is.
        onIncomingCall: {
            if (pageStack.currentPage.objectName !== "callPage") {
                pageStack.push(Qt.resolvedUrl("pages/CallPage.qml"))
            }
            activation.raiseWindow()

            callNotification.summary = !settings.notificationPreview
                                       ? qsTr("Incoming call")
                                       : matrix.calls.videoOffered
                                       || matrix.calls.videoRefused
                                       ? qsTr("Incoming video call")
                                       : qsTr("Incoming call")
            // Who is calling only where the user asked for message text: the
            // banner shows on the lock screen either way.
            callNotification.body = settings.notificationPreview ? peer : ""
            callNotification.previewSummary = callNotification.summary
            callNotification.previewBody = callNotification.body
            callNotification.publish()
            ringer.play()
        }

        // Answered, declined, or the caller gave up: the ring stops with the
        // ringing state, whichever way it ended.
        onStateChanged: {
            if (matrix.calls.state !== "ringing") {
                ringer.stop()
                callNotification.close()
            }
        }
    }

    Connections {
        target: matrix

        onSessionChanged: {
            app.showPageFor(matrix.sessionState)
            if (matrix.sessionState === "signed-in") {
                matrix.refreshEncryptionStatus()
                matrix.refreshStorageStatus()
                // A share that arrived while the session was still being
                // restored now has a room list to be offered.
                app.deliverShare()
            } else {
                // A sign-out ends the run as far as this is concerned: the
                // next session is a new device and gets asked again.
                app.securityShown = false
            }
        }

        // Both jobs in one handler each: QML refuses a type that binds the
        // same signal twice, and the page then fails to load.
        onEncryptionChanged: app.maybeShowSecurity()

        onStorageChanged: {
            // The gate can open or close under the running app — the retry on
            // the blocked page asks the secrets storage again.
            app.showPageFor(matrix.sessionState)
            app.maybeShowSecurity()
        }

        // The login itself happens in the browser; the core is waiting on a
        // loopback listener for the redirect.
        onLoginUrlReady: Qt.openUrlExternally(url)

        // A room read elsewhere clears the counter here too, and the banner goes with
        // it. Only for this room: the notification object is a single one.
        onRoomRead: {
            if (roomId === app.notifiedRoomId) {
                notification.close()
                app.notifiedRoomId = ""
            }
        }

        onRoomActivity: {
            notification.close()
            app.notifiedRoomId = roomId
            // The room's name is content - in a direct chat it is the other person. Same
            // switch as the text, because the banner shows on the lock screen.
            notification.summary = settings.notificationPreview
                                   ? roomName : qsTr("New message")
            // The count is the default, the message only where the user allowed it.
            // Non-text events are named by kind, and an undecryptable one says so.
            var preview = settings.notificationPreview ? app.previewLine(previewKind, previewText) : ""
            notification.body = preview.length > 0 ? preview
                    : mentions > 0
                      ? qsTr("%n mention(s)", "", mentions)
                      : qsTr("%n new message(s)", "", unread)
            // summary and body alone fill only the event feed; the banner that slides in
            // is the preview pair.
            notification.previewSummary = notification.summary
            notification.previewBody = notification.body
            notification.itemCount = unread
            notification.publish()
        }

        // A direct chat is requested from several places, so the navigation lives
        // here - on the room list page it was dead when the user came via spaces.
        onDirectChatReady: {
            if (roomId.length > 0) {
                pageStack.push(Qt.resolvedUrl("pages/RoomPage.qml"),
                               { roomId: roomId, roomName: "" })
            }
        }

        // A followed upgrade for the same reason: the old room may have been reached
        // from the chat list, a space or a search.
        onSuccessorReady: {
            if (roomId.length > 0) {
                pageStack.push(Qt.resolvedUrl("pages/RoomPage.qml"),
                               { roomId: roomId, roomName: "" })
            }
        }

        // A freshly created room opens right away. Its name comes back with the reply,
        // so the header is right before the first diff arrives.
        onRoomCreated: {
            if (roomId.length > 0) {
                pageStack.push(Qt.resolvedUrl("pages/RoomPage.qml"),
                               { roomId: roomId, roomName: name,
                                 encrypted: encrypted })
            }
        }

        // A verification request is time limited, so it takes the screen
        // rather than waiting to be discovered in a menu.
        onVerificationChanged: {
            if (matrix.verificationState === "requested"
                    && pageStack.currentPage.objectName !== "verificationPage") {
                pageStack.push(Qt.resolvedUrl("pages/VerificationPage.qml"))
            }
        }
    }
}
