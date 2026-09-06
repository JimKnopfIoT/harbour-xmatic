import QtQuick 2.0
import Sailfish.Silica 1.0

// A poll inside a message bubble: question, one line per answer, and a bar
// under each. The width comes from the caller and is handed down - a child
// here never measures its parent.
Column {
    id: block

    /// The row's `poll` field, as the core built it. Null for every other row.
    property var poll

    /// Which event the votes relate to.
    property string eventId

    /// Whether the poll is ours, which is the only case the end action appears
    /// in. The server checks power levels, not authorship - see docs.
    property bool own: false

    /// Set by the caller; nothing in here reads the parent's width.
    property real availableWidth: 0

    /// Still a local echo: no event id, so nothing to vote on yet.
    property bool pending: false

    readonly property bool ended: !!poll && poll.ended === true
    /// An undisclosed poll that is still running: the core sends no counts at
    /// all then, so there is nothing here to reveal.
    readonly property bool hidden: !!poll && poll.hidden === true
    readonly property var answers: poll ? (poll.answers || []) : []
    /// What the row says we chose. `chosen()` prefers the tap that is still
    /// on its way, so two quick taps do not both read the state before either.
    readonly property var mine: poll ? (poll.mine || []) : []
    property var localMine: null
    // Dropped only when the row's own selection really changed: any other
    // change to the row re-delivers the same `poll` object.
    property string mineKey: JSON.stringify(mine)
    onMineKeyChanged: localMine = null

    // A tap the row never confirms - a vote the poll ignored - is not kept.
    Timer {
        id: localMineTimer

        interval: 10000
        onTriggered: block.localMine = null
    }

    Connections {
        target: matrix.polls
        onVoteFailed: {
            if (eventId === block.eventId) {
                block.localMine = null
            }
        }
    }
    readonly property int maxSelections: poll && poll.maxSelections > 0
                                         ? poll.maxSelections : 1

    /// The text a client without poll support shows for the end event. It
    /// travels into the room, so it is written in the ending user's language.
    readonly property string endedText: qsTr("The poll has ended.")

    width: availableWidth
    spacing: Theme.paddingSmall

    function selection() {
        return block.localMine !== null ? block.localMine : block.mine
    }

    function chosen(answerId) {
        return block.selection().indexOf(answerId) >= 0
    }

    /// One response carries the whole selection: the spec counts the latest one
    /// per person, so a changed vote is the full list again.
    function vote(answerId) {
        if (block.ended || block.pending) {
            return
        }
        var current = block.selection()
        var next = []
        if (block.maxSelections <= 1) {
            next = block.chosen(answerId) ? [] : [answerId]
        } else {
            for (var i = 0; i < current.length; i++) {
                if (current[i] !== answerId) {
                    next.push(current[i])
                }
            }
            if (!block.chosen(answerId)) {
                // At the limit the oldest choice makes room; the row shows it.
                if (next.length >= block.maxSelections) {
                    next.shift()
                }
                next.push(answerId)
            }
        }
        block.localMine = next
        localMineTimer.restart()
        matrix.polls.vote(block.eventId, next)
    }

    Label {
        width: parent.width
        wrapMode: Text.Wrap
        textFormat: Text.PlainText
        font.pixelSize: Theme.fontSizeSmall
        color: Theme.primaryColor
        text: block.poll ? block.poll.question : ""
    }

    Repeater {
        model: block.answers

        Item {
            id: answerRow

            width: block.width
            height: answerLabel.height + Theme.paddingSmall / 2 + track.height

            Label {
                id: answerLabel

                anchors {
                    left: parent.left
                    right: percentLabel.left
                    rightMargin: block.hidden ? 0 : Theme.paddingMedium
                    top: parent.top
                }
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeSmall
                color: block.chosen(modelData.id) ? Theme.primaryColor
                                                  : Theme.secondaryColor
                // The own answer carries the mark, so the choice is legible
                // where a bar cannot say it - a hidden poll has none.
                text: (block.chosen(modelData.id) ? "✓ " : "") + modelData.text
            }

            Label {
                id: percentLabel

                anchors {
                    right: parent.right
                    baseline: answerLabel.baseline
                }
                visible: !block.hidden
                // An anchor does not see `visible`: without this the hidden
                // poll keeps a hole the width of "0 %".
                width: block.hidden ? 0 : implicitWidth
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("%1 %").arg(Math.round((modelData.share || 0) * 100))
            }

            // The bar. Not drawn at all while the counts are withheld: an empty
            // track would still be a place where a number is missing.
            Rectangle {
                id: track

                anchors {
                    left: parent.left
                    right: parent.right
                    bottom: parent.bottom
                }
                visible: !block.hidden
                height: block.chosen(modelData.id) ? Math.max(2, Theme.paddingSmall / 3)
                                                   : Math.max(1, Theme.paddingSmall / 5)
                radius: height / 2
                color: Theme.rgba(Theme.secondaryColor, 0.25)

                Rectangle {
                    anchors { left: parent.left; top: parent.top; bottom: parent.bottom }
                    width: parent.width * Math.max(0, Math.min(1, modelData.share || 0))
                    radius: parent.radius
                    color: Theme.highlightColor
                }
            }

            MouseArea {
                anchors.fill: parent
                enabled: !block.ended && !block.pending
                onClicked: block.vote(modelData.id)
            }
        }
    }

    // Why there is nothing to see yet. Without it the rows read as a poll
    // nobody has voted in.
    Label {
        width: parent.width
        visible: block.hidden
        wrapMode: Text.Wrap
        font.pixelSize: Theme.fontSizeExtraSmall
        font.italic: true
        color: Theme.secondaryColor
        text: qsTr("Results are shown once the poll ends")
    }

    Item {
        width: parent.width
        height: Math.max(votesLabel.height, stateLabel.height)

        Label {
            id: votesLabel

            anchors.left: parent.left
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryColor
            // Votes, not people: with several answers allowed one person casts more
            // than one, and the label says what is counted.
            // A floor where the server read hit its bound, and said so.
            text: (block.poll && block.poll.complete === false ? "≥ " : "")
                  + qsTr("%n vote(s)", "", block.poll ? (block.poll.votes || 0) : 0)
        }

        Label {
            id: stateLabel

            anchors.right: parent.right
            font.pixelSize: Theme.fontSizeExtraSmall
            color: block.ended ? Theme.highlightColor : Theme.secondaryColor
            text: {
                if (block.ended) {
                    return qsTr("Ended")
                }
                if (block.maxSelections > 1) {
                    return qsTr("Several answers")
                }
                return ""
            }
        }
    }

    // On the entry itself, never on the row above it: a poll of ours that has
    // ended keeps its numbers, it only loses this.
    Label {
        anchors.right: parent.right
        visible: block.own && !block.ended && !block.pending
        font.pixelSize: Theme.fontSizeExtraSmall
        color: Theme.highlightColor
        text: qsTr("End poll")

        MouseArea {
            // Wider than the small label, not narrower: a link that misses
            // its tap reads as a button that does nothing.
            anchors.fill: parent
            anchors.margins: -Theme.paddingMedium
            enabled: !block.pending
            onClicked: matrix.polls.end(block.eventId, block.endedText)
        }
    }
}
