import QtQuick 2.0
import Sailfish.Silica 1.0

// Profile of one member in one room. Loads via `member.profile`, reloads on
// `memberChanged` (no diff follows core-made state changes). Actions as
// buttons, moderation in the pull-down; visibility per entry, never on a
// container.
Page {
    id: page

    property string roomId
    property string userId

    // The `member.profile` answer, or null while loading.
    property var profile: null
    readonly property bool loaded: profile !== null
    // Page-local, because matrix.lastError is global: a slow download
    // elsewhere would otherwise read as a failed ban.
    property string error: ""
    readonly property bool failed: error.length > 0 && !loaded

    // Remorse guard as in MemberListPage: abort on minimise, never fire on
    // an inactive page.
    property var activeRemorse: null
    readonly property bool appForeground: Qt.application.active
    onAppForegroundChanged: {
        if (!appForeground && activeRemorse && activeRemorse.pending) {
            activeRemorse.cancel()
        }
    }

    allowedOrientations: Orientation.All

    function reload() {
        error = ""
        matrix.loadMemberProfile(roomId, userId)
    }

    Component.onCompleted: reload()

    function guardedRemorse(text, action) {
        page.activeRemorse = Remorse.popupAction(page, text, function() {
            page.activeRemorse = null
            if (page.status !== PageStatus.Active || !Qt.application.active) {
                return
            }
            action()
        })
    }

    // The picture at full size. The list and the row show a thumbnail; this
    // asks for the original under a key of its own, so both stay cached.
    // A profile picture is never encrypted, so a bare `url` source is enough.
    function openAvatar() {
        if (!loaded || !profile.avatar) {
            return
        }
        var key = page.userId + "/avatar-full"
        var known = matrix.mediaPath(key)
        matrix.requestMedia(key, { "url": profile.avatar }, false)
        pageStack.push(Qt.resolvedUrl("ImageViewPage.qml"), {
                           mediaKey: key,
                           source: known.length > 0 ? "file://" + known : "",
                           fileName: "",
                           mimeType: "image/jpeg"
                       })
    }

    // A copy, so assigning it back counts as a change: QML compares by
    // reference, and mutating the object in place updates no binding.
    function withField(key, value) {
        var copy = {}
        for (var name in profile) {
            copy[name] = profile[name]
        }
        copy[key] = value
        return copy
    }

    // 0 member, 50 moderator, 100 admin — the same reading as the list rows.
    function roleText() {
        if (!loaded) {
            return ""
        }
        if (profile.power >= 100) {
            return qsTr("Admin")
        }
        if (profile.power >= 50) {
            return qsTr("Moderator")
        }
        return qsTr("Member")
    }

    function membershipText() {
        if (!loaded) {
            return ""
        }
        if (profile.membership === "invite") {
            return qsTr("invited")
        }
        if (profile.membership === "ban") {
            return qsTr("banned")
        }
        if (profile.membership === "leave") {
            return qsTr("left the room")
        }
        return ""
    }

    Connections {
        target: matrix

        onMemberProfileReady: {
            if (profileData.roomId === page.roomId
                    && profileData.userId === page.userId) {
                page.profile = profileData
                page.error = ""
            }
        }

        onMemberProfileFailed: page.error = message

        // The result is applied from the answer, never by re-reading: the
        // server has confirmed the change, but the local store only learns of
        // the state event on a later sync, so a reload would hand back the
        // values from before the action.
        onMemberActionDone: {
            if (!page.loaded || result.userId !== page.userId) {
                return
            }
            if (action === "ban") {
                page.profile = page.withField("membership", "ban")
                page.profile = page.withField("canBan", false)
                page.profile = page.withField("canRemove", false)
                page.profile = page.withField("canUnban", true)
            } else if (action === "unban") {
                page.profile = page.withField("membership", "leave")
                page.profile = page.withField("canUnban", false)
                page.profile = page.withField("canBan", true)
            } else if (action === "setPower") {
                page.profile = page.withField("power", result.power)
            } else if (action === "setIgnored") {
                page.profile = page.withField("ignored", result.ignored)
            } else if (action === "withdrawVerification") {
                page.profile = page.withField("verification", "unverified")
            }
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        // Moderation; flags come with the profile. Removing pops the page.
        // The reload entry has no condition on purpose: without it the pulley
        // is empty on one's own profile and in the failed state, and an empty
        // pulley is a gesture that does nothing.
        PullDownMenu {
            MenuItem {
                text: qsTr("Reload")
                onClicked: page.reload()
            }

            MenuItem {
                text: qsTr("Make moderator")
                visible: page.loaded && profile.canSetPower && profile.power < 50
                         && profile.ownPower >= 50
                onClicked: matrix.setMemberPower(page.roomId, page.userId, 50)
            }

            MenuItem {
                text: qsTr("Make admin")
                visible: page.loaded && profile.canSetPower && profile.power < 100
                         && profile.ownPower >= 100
                onClicked: {
                    var dialog = pageStack.push(
                                Qt.resolvedUrl("ConfirmDialog.qml"),
                                {
                                    question: qsTr("Really make this member an admin?"),
                                    subject: profile.displayName || page.userId,
                                    explanation: qsTr("This cannot be taken back: only they themselves can step down afterwards."),
                                    acceptLabel: qsTr("Make admin")
                                })
                    dialog.accepted.connect(function() {
                        matrix.setMemberPower(page.roomId, page.userId, 100)
                    })
                }
            }

            MenuItem {
                text: qsTr("Demote to member")
                visible: page.loaded && profile.canSetPower && profile.power > 0
                onClicked: matrix.setMemberPower(page.roomId, page.userId, 0)
            }

            MenuItem {
                text: profile !== null && profile.membership === "invite"
                      ? qsTr("Revoke invitation") : qsTr("Remove from room")
                visible: page.loaded && profile.canRemove
                onClicked: page.guardedRemorse(qsTr("Removing"), function() {
                    matrix.removeMember(page.roomId, page.userId)
                    pageStack.pop()
                })
            }

            MenuItem {
                text: qsTr("Ban from room")
                visible: page.loaded && profile.canBan
                onClicked: {
                    var dialog = pageStack.push(
                                Qt.resolvedUrl("ConfirmDialog.qml"),
                                {
                                    question: qsTr("Really ban this member?"),
                                    subject: profile.displayName || page.userId,
                                    explanation: qsTr("%1 is removed from the room and cannot come back until the ban is lifted.").arg(page.userId),
                                    acceptLabel: qsTr("Ban")
                                })
                    dialog.accepted.connect(function() {
                        page.guardedRemorse(qsTr("Banning"), function() {
                            matrix.banMember(page.roomId, page.userId)
                        })
                    })
                }
            }

            MenuItem {
                text: qsTr("Lift ban")
                visible: page.loaded && profile.canUnban
                onClicked: matrix.unbanMember(page.roomId, page.userId)
            }
        }

        Column {
            id: content

            width: parent.width

            PageHeader {
                title: page.loaded && profile.displayName.length > 0
                       ? profile.displayName : page.userId
            }

            // Failed actions would otherwise fail silently.
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

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Large
                running: !page.loaded && !page.failed
                visible: running
            }

            Avatar {
                id: profileAvatar

                anchors.horizontalCenter: parent.horizontalCenter
                size: Theme.itemSizeHuge
                source: page.loaded && profile.avatar ? profile.avatar : ""
                name: page.loaded && profile.displayName.length > 0
                      ? profile.displayName : page.userId
                visible: page.loaded

                MouseArea {
                    anchors.fill: parent
                    // Only where there is a picture: the initials have no
                    // full size to show.
                    enabled: page.loaded && profile.avatar
                    onClicked: page.openAvatar()
                }
            }

            Item {
                width: 1
                height: Theme.paddingMedium
            }

            // Tap copies the address; hint below confirms.
            Label {
                anchors.horizontalCenter: parent.horizontalCenter
                width: Math.min(implicitWidth,
                                parent.width - 2 * Theme.horizontalPageMargin)
                truncationMode: TruncationMode.Fade
                horizontalAlignment: Text.AlignHCenter
                color: addressArea.pressed ? Theme.secondaryHighlightColor
                                           : Theme.highlightColor
                textFormat: Text.PlainText
                text: page.userId
                visible: page.loaded

                MouseArea {
                    id: addressArea
                    anchors {
                        fill: parent
                        margins: -Theme.paddingLarge
                    }
                    onClicked: {
                        Clipboard.text = page.userId
                        copiedHint.opacity = 1.0
                        copiedTimer.restart()
                    }
                }
            }

            Label {
                id: copiedHint
                anchors.horizontalCenter: parent.horizontalCenter
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                text: qsTr("Address copied")
                opacity: 0.0
                visible: opacity > 0
                Behavior on opacity { FadeAnimation { } }

                Timer {
                    id: copiedTimer
                    interval: 2000
                    onTriggered: copiedHint.opacity = 0.0
                }
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }

            DetailItem {
                label: qsTr("Role")
                value: page.membershipText().length > 0
                       ? page.roleText() + " · " + page.membershipText()
                       : page.roleText()
                visible: page.loaded
            }

            DetailItem {
                label: qsTr("Member since")
                value: page.loaded && profile.sinceMs > 0
                       ? Format.formatDate(new Date(profile.sinceMs), Formatter.DateLong)
                       : ""
                visible: page.loaded && profile.sinceMs > 0
            }

            DetailItem {
                label: qsTr("Invited by")
                value: page.loaded ? profile.invitedBy : ""
                visible: page.loaded && profile.invitedBy.length > 0
            }

            DetailItem {
                label: qsTr("Devices")
                value: page.loaded ? profile.devices : 0
                visible: page.loaded && profile.devices > 0
            }

            // Identity states: verified / violation (was verified, keys
            // changed) / unverified; "unknown" hides the block.
            SectionHeader {
                text: qsTr("Encryption")
                visible: page.loaded && !profile.isSelf
                         && profile.verification !== "unknown"
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                visible: page.loaded && !profile.isSelf
                         && profile.verification !== "unknown"
                color: {
                    if (!page.loaded) {
                        return Theme.secondaryColor
                    }
                    if (profile.verification === "violation") {
                        return Theme.errorColor
                    }
                    if (profile.verification === "verified") {
                        return Theme.highlightColor
                    }
                    return Theme.secondaryColor
                }
                text: {
                    if (!page.loaded) {
                        return ""
                    }
                    if (profile.verification === "verified") {
                        return qsTr("Identity verified")
                    }
                    if (profile.verification === "violation") {
                        return qsTr("The identity has changed since it was verified. Verify again, or withdraw the verification.")
                    }
                    return qsTr("Identity not verified")
                }
            }

            Item {
                width: 1
                height: Theme.paddingMedium
                visible: page.loaded && !profile.isSelf
            }

            ButtonLayout {
                visible: page.loaded && !profile.isSelf

                Button {
                    text: qsTr("Send direct message")
                    onClicked: matrix.startDirectChat(page.userId)
                }

                Button {
                    text: qsTr("Verify")
                    visible: page.loaded
                             && (profile.verification === "unverified"
                                 || profile.verification === "violation")
                    onClicked: {
                        matrix.requestVerification(page.userId)
                        pageStack.push(Qt.resolvedUrl("VerificationPage.qml"))
                    }
                }

                Button {
                    text: qsTr("Withdraw verification")
                    visible: page.loaded && profile.verification === "violation"
                    onClicked: matrix.withdrawMemberVerification(page.userId)
                }

                Button {
                    // The allow list from the privacy page. Kept on this
                    // device, unlike ignoring, which is the account's.
                    text: matrix.callerAllowed(page.userId)
                          ? qsTr("Forbid calls") : qsTr("Allow calls")
                    visible: page.loaded && !profile.isSelf
                    onClicked: {
                        if (matrix.callerAllowed(page.userId)) {
                            matrix.forbidCaller(page.userId)
                        } else {
                            matrix.allowCaller(page.userId)
                        }
                    }
                }

                Button {
                    // Ignoring is account-wide.
                    text: page.loaded && profile.ignored
                          ? qsTr("Stop ignoring") : qsTr("Ignore")
                    onClicked: {
                        if (profile.ignored) {
                            matrix.setMemberIgnored(page.userId, false)
                        } else {
                            page.guardedRemorse(qsTr("Ignoring"), function() {
                                matrix.setMemberIgnored(page.userId, true)
                            })
                        }
                    }
                }
            }

            SectionHeader {
                text: qsTr("Shared rooms")
                visible: page.loaded && !profile.isSelf
            }

            // Display only, deliberately not tappable.
            Repeater {
                model: page.loaded ? profile.sharedRooms : []

                Label {
                    x: Theme.horizontalPageMargin
                    width: content.width - 2 * Theme.horizontalPageMargin
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.primaryColor
                    textFormat: Text.PlainText
                    text: modelData
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryColor
                visible: page.loaded && !profile.isSelf
                         && profile.sharedRooms.length === 0
                text: qsTr("No other shared rooms")
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }

        VerticalScrollDecorator { }
    }
}
