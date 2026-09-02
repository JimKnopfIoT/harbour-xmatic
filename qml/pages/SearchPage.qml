import QtQuick 2.0
import Sailfish.Silica 1.0

// Message search inside one room. The index is local, so a room never
// scrolled back finds only recent messages - the placeholder says so.
Page {
    id: page

    property string roomId: ""
    property string roomName: ""

    allowedOrientations: Orientation.All

    // Typing re-searches after a pause: each search is an index lookup plus one
    // event load per row.
    Timer {
        id: searchDelay

        interval: 400
        onTriggered: matrix.searchRoom(page.roomId, page.pattern)
    }

    // Mirrored out of the header, which is created late: a sibling must not
    // reach into it.
    property string pattern: ""
    property string failure: ""

    Connections {
        target: matrix
        onSearchFailed: page.failure = error
        // Everything on the device is in the index now. A query typed while that ran
        // searched a part of the room, so it is asked again.
        onIndexReady: {
            if (page.pattern.length > 0) {
                matrix.searchRoom(page.roomId, page.pattern)
            }
        }
    }

    // The SDK indexes an event when it saves it, so history older than the feature
    // would find nothing. Handing it over is local and costs no request.
    Component.onCompleted: matrix.indexRoom(roomId)

    // The results belong to this page. Leaving it without clearing would leave
    // them standing for the next room that opens a search.
    Component.onDestruction: matrix.clearSearch()

    SilicaListView {
        id: hitList

        // Without this the view hands focus to row 0 on every model reset and closes
        // the keyboard mid-typing. Every result page is a reset.
        currentIndex: -1

        anchors.fill: parent
        model: matrix.searchResults

        header: Column {
            width: hitList.width

            PageHeader {
                title: qsTr("Search messages")
                description: page.roomName
            }

            SearchField {
                width: parent.width
                placeholderText: qsTr("Search this conversation")
                // The keyboard's word list is a store outside this sandbox and suggests what
                // it learned elsewhere. What is typed here is private conversation.
                inputMethodHints: Qt.ImhNoPredictiveText
                                  | Qt.ImhSensitiveData
                                  | Qt.ImhNoAutoUppercase
                onTextChanged: {
                    page.pattern = text
                    page.failure = ""
                    searchDelay.restart()
                }
                Component.onCompleted: forceActiveFocus()
            }

            // The empty state belongs under the field: Silica's ViewPlaceholder centres
            // itself in the viewport and landed on top of the title and the field.
            Column {
                width: parent.width
                visible: hitList.count === 0 && !matrix.searching
                         && !matrix.indexing
                spacing: Theme.paddingMedium

                Item {
                    width: 1
                    height: Theme.paddingLarge
                }

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeLarge
                    color: Theme.secondaryHighlightColor
                    text: page.failure.length > 0
                          ? qsTr("Search failed")
                          : (page.pattern.length === 0 ? "" : qsTr("Nothing found"))
                }

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.secondaryColor
                    text: page.failure.length > 0
                          ? page.failure
                          : qsTr("Whole words only: \"test\" does not find \"test9\". Pictures, files and messages from bots are not searched. And only what this device has already downloaded - load older messages in the conversation to add more.")
                }
            }
        }

        delegate: ListItem {
            id: hit

            width: hitList.width
            contentHeight: hitColumn.height + 2 * Theme.paddingMedium

            onClicked: {
                var room = pageStack.find(function(candidate) {
                    return candidate.objectName === "roomPage"
                })
                if (room && room.jumpToEvent) {
                    room.jumpToEvent(model.eventId)
                    pageStack.pop(room)
                    return
                }
                pageStack.pop()
            }

            Column {
                id: hitColumn

                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                anchors.verticalCenter: parent.verticalCenter
                spacing: Theme.paddingSmall

                Row {
                    width: parent.width
                    spacing: Theme.paddingMedium

                    Label {
                        width: parent.width - stamp.width - Theme.paddingMedium
                        text: model.senderName
                        truncationMode: TruncationMode.Fade
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: hit.highlighted ? Theme.highlightColor
                                               : Theme.secondaryHighlightColor
                    }

                    Label {
                        id: stamp

                        // Same rule as a room row: relative inside this year, dated beyond it. A hit
                        // is usually old, and "three months ago" is not a date one can place.
                        text: {
                            if (!(model.timestamp > 0)) {
                                return ""
                            }
                            var when = new Date(model.timestamp)
                            return Format.formatDate(
                                        when,
                                        when.getFullYear() === new Date().getFullYear()
                                        ? Formatter.TimepointRelative
                                        : Formatter.DateMedium)
                        }
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.secondaryColor
                    }
                }

                Label {
                    width: parent.width
                    text: model.body
                    wrapMode: Text.Wrap
                    maximumLineCount: 3
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                    font.pixelSize: Theme.fontSizeSmall
                    color: hit.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
            }
        }

        // Reaching the end asks for the next page, the same way the room list
        // grows. Gated on the search's own state, never on the global `busy`.
        onAtYEndChanged: {
            if (atYEnd && matrix.searchHasMore && !matrix.searching) {
                matrix.searchMore()
            }
        }

        VerticalScrollDecorator { }
    }

    BusyIndicator {
        anchors.centerIn: parent
        size: BusyIndicatorSize.Large
        running: (matrix.searching || matrix.indexing) && hitList.count === 0
    }
}
