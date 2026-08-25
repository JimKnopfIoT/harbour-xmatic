import QtQuick 2.0
import Sailfish.Silica 1.0
import QtMultimedia 5.6
import Nemo.Notifications 1.0
import Nemo.KeepAlive 1.2
import Sailfish.Share 1.0

import "pages"

ApplicationWindow {
    id: app

    // Which session state the currently shown page was built for, so the page
    // is only exchanged when the state actually changes.
    property string shownState: "none"

    // The room the standing notification is about, or "" if none stands.
    property string notifiedRoomId: ""

    initialPage: Component { LoginPage { } }
    cover: Qt.resolvedUrl("cover/CoverPage.qml")

    // Silica's default is Text.AutoText, and every Label inherits it — a
    // PageHeader title, a DetailItem, an error line. Display names, room
    // names and server error texts all pass through those, so a name of
    // `<img src=http://…>` fetched a remote picture on sight. Only the
    // message body opts back out, explicitly, after escaping (RoomPage's
    // linkifyBody); an explicit binding beats this default.
    _defaultLabelFormat: Text.PlainText
    // Do not try to widen this through `defaultAllowedOrientations` — that one
    // is read-only in Silica, and assigning it makes the whole window fail to
    // load (a white screen, with nothing but a warning in the journal). Pages
    // this app does not own, such as the attachment picker, set their own
    // orientation at the call site instead.
    allowedOrientations: defaultAllowedOrientations

    // The root page a replaceAbove is still owed, or "" if none is pending.
    // Signing out fires while the sign-out dialog is still animating, and Silica
    // silently drops a page push issued during a transition ("cannot push while
    // transition is in progress") — the login page then never appears and the
    // app is a dead end until it is restarted. So the switch is deferred until
    // the stack is idle.
    property string pendingRoot: ""

    // What another application shared with xmatic, until there is somewhere to
    // put it: { body, path, mimeType }, or null. A share can arrive before the
    // session is restored — sharing may be what started the app in the first
    // place — and pushing a page while the stack is still animating is dropped
    // silently, so the item waits here and every state change tries again.
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
        // An encrypted session whose key is not at hand is not "no session":
        // its page retries the unlock, and never leads into a login that
        // would clear the store.
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
                || root === Qt.resolvedUrl("pages/SessionLockedPage.qml") ? {} : { isHome: true }
    }

    function showPageFor(state) {
        if (state === shownState) {
            return
        }
        shownState = state
        if (pageStack.busy) {
            pendingRoot = rootFor(state)
        } else {
            pageStack.replaceAbove(null, rootFor(state), propsFor(rootFor(state)))
        }
    }

    Connections {
        target: pageStack
        // One handler, both jobs: QML refuses a type that binds the same signal
        // twice, and the page then fails to load with nothing but "could not
        // load page" to go on.
        onBusyChanged: {
            if (!pageStack.busy && app.pendingRoot !== "") {
                var root = app.pendingRoot
                app.pendingRoot = ""
                pageStack.replaceAbove(null, root, app.propsFor(root))
            }
            if (!pageStack.busy) {
                app.deliverShare()
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

    Component.onCompleted: matrix.restoreSession()

    // The tap on a notification, and the launcher's hand-over, arrive here.
    // Neither carries an argument: which room is meant is this app's own
    // knowledge (`notifiedRoomId`), never the caller's claim.
    Connections {
        target: activation

        onRaiseRequested: app.activate()

        onNotifiedRoomRequested: {
            app.activate()
            app.openNotifiedRoom()
        }
    }

    // Opens the room the standing notification is about, once. Nothing stands
    // -> nothing happens, so a repeated call cannot walk the user through
    // rooms. Not while the session is still being restored either: the room
    // list is not there yet, and the login page is what belongs on screen.
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
        // Coming from another room, the new one takes its place instead of
        // stacking on it: swiping back belongs in the chat list, not in the
        // room the notification pulled the user out of.
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
        // Without a category a notification is silent: the sound, the vibration
        // and the LED pattern all hang off the category, not off the
        // notification itself. This one is the system's instant-messaging
        // category — its `chat_exists` feedback plays the IM tone the user set,
        // buzzes once and lights the communication LED, and it turns the
        // display on, which is what every other messenger on the device does.
        category: "x-nemo.messaging.im"
        // The category brings its own icon (the system's message glyph), so
        // the app's own has to be named or the banner would not say who is
        // talking.
        appIcon: "/usr/share/icons/hicolor/86x86/apps/harbour-xmatic.png"
        isTransient: false
        // Tapping opens the room this banner is about. The method takes no
        // argument - see src/appservice.h for why - so nothing about the room
        // travels through the notification, and nothing of it stays behind in
        // the system's notification store.
        remoteActions: [{
            "name": "default",
            "service": "org.xmatic.xmatic",
            "path": "/org/xmatic/xmatic",
            "iface": "org.xmatic.xmatic",
            "method": "openNotified"
        }]
    }

    // Incoming shares from other applications — a link from the browser, a
    // picture from the gallery. The share dialog finds xmatic through the
    // X-Share Method block in the desktop file and calls this object over
    // D-Bus; the name it registers is the one Sailjail assigns the app.
    ShareProvider {
        method: "room"
        // The application owns its D-Bus name itself; without this the object
        // is registered but the name is not, and the share dialog's call
        // starts a second copy of the app through the service file instead of
        // reaching the running one.
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
                // Whatever asked for the share names the file; a path
                // inside this app's own directories is refused, because the
                // session token and the crypto store live there.
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

    // Sailfish suspends idle devices. Without a scheduled wake-up the sync
    // connection is dropped while the screen is off and messages only arrive
    // when the phone is picked up again.
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

    // A ringing call is not a message: it rings until it is answered or the
    // caller gives up. The system has no category for that, so the app rings
    // itself - the notification's own tone plays four times and stops, which
    // is exactly what was missed.
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
                // A share that arrived while the session was still being
                // restored now has a room list to be offered.
                app.deliverShare()
            }
        }

        // The login itself happens in the browser; the core is waiting on a
        // loopback listener for the redirect.
        onLoginUrlReady: Qt.openUrlExternally(url)

        // New messages in a room that is not on screen become a system
        // notification. The room list already carries the unread counts, so
        // this needs no extra request.
        // A room read on another device clears the counter here too, and the
        // banner has to go with it — the LED, the event feed entry and the
        // banner are one published notification, so closing it takes all
        // three down. Only when it is this room's: the notification object is
        // a single one, and the room it last announced is the only one it
        // could be showing.
        onRoomRead: {
            if (roomId === app.notifiedRoomId) {
                notification.close()
                app.notifiedRoomId = ""
            }
        }

        onRoomActivity: {
            notification.close()
            app.notifiedRoomId = roomId
            // The room's name is content: in a direct chat it is the other
            // person. Behind the same switch as the message text, because the
            // banner shows on the lock screen.
            notification.summary = settings.notificationPreview
                                   ? roomName : qsTr("New message")
            // The count is the default; the message itself only when the user
            // switched that on (Account → This app), because the banner also
            // shows on the lock screen. Non-text events are named by kind, so
            // a picture says "picture" in the UI's language, and an event this
            // device could not decrypt says so instead of pretending to be a
            // count.
            var preview = settings.notificationPreview ? app.previewLine(previewKind, previewText) : ""
            notification.body = preview.length > 0 ? preview
                    : mentions > 0
                      ? qsTr("%n mention(s)", "", mentions)
                      : qsTr("%n new message(s)", "", unread)
            // summary and body alone only fill the event feed. The banner that
            // slides in over whatever is on screen is the preview pair, and it
            // is also what makes the arrival noticeable at all when the phone
            // is in hand.
            notification.previewSummary = notification.summary
            notification.previewBody = notification.body
            notification.itemCount = unread
            notification.publish()
        }

        // A direct chat can be requested from several places — the new-chat
        // dialog and the member list — so the navigation lives here, where it
        // works no matter which pages happen to exist. (On the room list page
        // it was dead whenever the user came in through the spaces.)
        onDirectChatReady: {
            if (roomId.length > 0) {
                pageStack.push(Qt.resolvedUrl("pages/RoomPage.qml"),
                               { roomId: roomId, roomName: "" })
            }
        }

        // Following a room upgrade lands here for the same reason: the old room
        // may have been reached from the chat list, a space or a search, and
        // the new one has to open from any of them.
        onSuccessorReady: {
            if (roomId.length > 0) {
                pageStack.push(Qt.resolvedUrl("pages/RoomPage.qml"),
                               { roomId: roomId, roomName: "" })
            }
        }

        // A freshly created room opens right away — creating one is the start
        // of writing in it. Its name comes back with the reply, so the header
        // is right before the first sync diff arrives. Encryption was decided
        // in the dialog and is already in place.
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
