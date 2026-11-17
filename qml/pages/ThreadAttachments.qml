import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import "Composing.js" as Composing

// Attachments for a thread: the room's picker and send page, sent on the thread's
// timeline. Every push waits until the page is on top again.
Item {
    id: attachments

    property Page page
    property Item composer
    property string roomId
    property string threadRoot
    property var unverifiedUsers: []

    /// What to do once the page is on top: `otherFiles` or `sendMedia`.
    property var pendingAction: null
    /// The files picked, until the send page can be opened over the thread.
    property var pendingAttachment: null

    function pick() {
        pageStack.push(picturePicker)
    }

    Connections {
        target: attachments.page
        onStatusChanged: {
            if (attachments.page.status !== PageStatus.Active || !attachments.pendingAction) {
                return
            }
            var action = attachments.pendingAction
            attachments.pendingAction = null
            if (action.kind === "otherFiles") {
                pageStack.push(multiContentPicker)
            } else if (action.kind === "sendMedia") {
                attachments.submit(action)
            }
        }
    }

    // A push during a transition is dropped; the attachment waits for the stack.
    Connections {
        target: pageStack
        onBusyChanged: {
            if (!pageStack.busy && attachments.deferred
                    && attachments.page.status === PageStatus.Active) {
                var action = attachments.deferred
                attachments.deferred = null
                attachments.submit(action)
            }
        }
    }

    /// Never dropped: the only copy of what the user chose.
    property var deferred: null

    function submit(action) {
        if (pageStack.busy) {
            attachments.deferred = action
            return
        }
        var pending = Composing.pendingUnverified(attachments.unverifiedUsers, matrix)
        if (pending.length > 0) {
            var dialog = pageStack.push(Qt.resolvedUrl("UnverifiedRecipientsDialog.qml"),
                                        { users: pending })
            if (!dialog) {
                attachments.deferred = action
                return
            }
            dialog.accepted.connect(function() { attachments.sendNow(action) })
            return
        }
        attachments.sendNow(action)
    }

    function sendNow(action) {
        // One event per file; caption and reply belong to the first, as in the room.
        for (var i = 0; i < action.files.length; i++) {
            var file = action.files[i]
            matrix.sendMedia(file.path, file.mimeType,
                             i === 0 ? action.caption : "",
                             i === 0 ? (action.replyTo || "") : "",
                             0, action.original === true,
                             i === 0 && action.mentions ? action.mentions : [],
                             attachments.threadRoot)
        }
        if (action.replyTo && action.replyTo === attachments.page.replyingEventId) {
            attachments.page.clearReplyState()
        }
        attachments.page.followTail = true
    }

    Component {
        id: picturePicker

        AttachmentPickerPage {
            // No draft-as-file here: a thread sends text as text.
            draftText: false

            onAccepted: {
                if (otherFilesWanted) {
                    attachments.pendingAction = { "kind": "otherFiles" }
                    return
                }
                if (picked.length > 0) {
                    attachments.pendingAttachment = picked
                    attachmentWait.restart()
                }
            }
        }
    }

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
                if (picked.length > 0) {
                    attachments.pendingAttachment = picked
                    attachmentWait.restart()
                }
            }
        }
    }

    Timer {
        id: attachmentWait

        property int tries: 0

        interval: 60
        repeat: true
        onTriggered: {
            if (!attachments.pendingAttachment) {
                stop()
                tries = 0
                return
            }
            if (attachments.page.status === PageStatus.Active) {
                stop()
                tries = 0
                var picked = attachments.pendingAttachment
                attachments.pendingAttachment = null
                // The typed text goes along as the caption, and comes back on cancel.
                var typed = attachments.composer.text
                var typedMentions = attachments.composer.mentionsPicked
                attachments.composer.clearField()
                attachments.composer.clearMentions()
                pageStack.push(Qt.resolvedUrl("SendMediaPage.qml"), {
                    files: picked,
                    caption: typed,
                    roomId: attachments.roomId,
                    mentionsPicked: typedMentions,
                    replyTo: attachments.page.replyingEventId,
                    replySender: attachments.page.replyingTo,
                    replyBody: attachments.page.replyingBody,
                    send: function (files, caption, replyTo, original, mentions) {
                        attachments.pendingAction = {
                            "kind": "sendMedia",
                            "files": files,
                            "caption": caption,
                            "mentions": mentions,
                            "replyTo": replyTo,
                            "original": original
                        }
                    },
                    afterCancel: function (text, mentions) {
                        attachments.composer.text = text
                        attachments.composer.mentionsPicked = mentions
                        for (var i = 0; i < picked.length; i++) {
                            matrix.cameraShots.discard(picked[i].path)
                        }
                    }
                })
                return
            }
            // Bounded: the picker did not come back by itself.
            if (++tries > 25) {
                tries = 0
                stop()
                attachments.pendingAttachment = null
                pageStack.pop(attachments.page)
            }
        }
    }
}
