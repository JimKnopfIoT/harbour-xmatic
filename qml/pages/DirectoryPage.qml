import QtQuick 2.0
import Sailfish.Silica 1.0

// A public room directory. The server to ask is a choice: the own homeserver,
// one of the big public directories, or a server the user added — the choice
// is persisted. Topic and member count are the preview; joining is a direct
// action with a remorse timer. Paging, ordering and deduplication come from
// the core's search task.
Page {
    id: page

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


    allowedOrientations: Orientation.All

    // Index 0 is always the own homeserver, taken from the session; the
    // search then asks it directly instead of going through federation.
    readonly property string ownServer: matrix.userId.substring(matrix.userId.indexOf(":") + 1)
    readonly property var knownDirectories: ["matrix.org", "matrixrooms.info", "hackint.org"]
    property var serverChoices: {
        var choices = [ownServer]
        var candidates = knownDirectories.concat(matrix.directoryServers)
        for (var i = 0; i < candidates.length; i++) {
            if (choices.indexOf(candidates[i]) < 0) {
                choices.push(candidates[i])
            }
        }
        return choices
    }
    // Holds the re-search back until the persisted choice is restored.
    property bool serverRestored: false
    // Choice and pattern, mirrored out of the header items: the header and
    // its children are created late and in order, so anything outside them —
    // including a sibling created earlier — must not reach in.
    property string selectedServer: ""
    property string searchPattern: ""
    // The combo box registers itself here — ids inside the header component
    // cannot be resolved from outside it, only the other way around.
    property Item serverBoxItem: null

    function research() {
        matrix.searchDirectory(searchPattern, selectedServer)
    }

    Component.onDestruction: matrix.stopDirectory()

    // Typing re-searches after a short pause instead of on every keystroke —
    // each search is a server round trip.
    Timer {
        id: searchDelay

        interval: 600
        onTriggered: page.research()
    }

    SilicaListView {
        id: resultList

        anchors.fill: parent
        model: matrix.directory

        PullDownMenu {
            MenuItem {
                // Only user-added servers can go away again.
                visible: matrix.directoryServers.indexOf(page.selectedServer) >= 0
                text: qsTr("Remove this server")
                onClicked: {
                    var name = page.selectedServer
                    if (page.serverBoxItem) {
                        page.serverBoxItem.currentIndex = 0
                    }
                    matrix.removeDirectoryServer(name)
                }
            }
            MenuItem {
                text: qsTr("Add directory server")
                onClicked: {
                    var dialog = pageStack.push(Qt.resolvedUrl("AddDirectoryServerDialog.qml"))
                    dialog.accepted.connect(function() {
                        matrix.addDirectoryServer(dialog.serverName)
                        var index = page.serverChoices.indexOf(dialog.serverName)
                        if (index >= 0 && page.serverBoxItem) {
                            page.serverBoxItem.currentIndex = index
                        }
                    })
                }
            }
        }

        header: Column {
            width: resultList.width

            PageHeader {
                title: qsTr("Discover rooms")
            }

            ComboBox {
                id: serverBox

                width: parent.width
                label: qsTr("Directory")
                // The menu may not be instantiated before it first opens, so
                // the shown value cannot rely on currentItem alone.
                value: page.serverChoices[currentIndex] || ""
                menu: ContextMenu {
                    Repeater {
                        model: page.serverChoices

                        MenuItem {
                            text: modelData
                        }
                    }
                }
                onCurrentIndexChanged: {
                    page.selectedServer = currentIndex <= 0
                            ? "" : page.serverChoices[currentIndex]
                    if (!page.serverRestored) {
                        return
                    }
                    matrix.directoryServer = page.selectedServer
                    page.research()
                }
                Component.onCompleted: {
                    page.serverBoxItem = serverBox
                    var saved = matrix.directoryServer
                    var index = saved.length > 0 ? page.serverChoices.indexOf(saved) : 0
                    currentIndex = index >= 0 ? index : 0
                    page.selectedServer = currentIndex <= 0
                            ? "" : page.serverChoices[currentIndex]
                    page.serverRestored = true
                    page.research()
                }
            }

            SearchField {
                id: searchField

                width: parent.width
                placeholderText: qsTr("Search the room directory")
                onTextChanged: {
                    page.searchPattern = text
                    searchDelay.restart()
                }
            }
        }

        delegate: ListItem {
            id: resultItem

            contentHeight: resultColumn.height + 2 * Theme.paddingMedium

            onClicked: openMenu()

            Column {
                id: resultColumn

                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                spacing: Theme.paddingSmall

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    color: resultItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                    textFormat: Text.PlainText
                    text: model.name
                }

                Label {
                    width: parent.width
                    visible: text.length > 0
                    maximumLineCount: 2
                    wrapMode: Text.Wrap
                    elide: Text.ElideRight
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: resultItem.highlighted ? Theme.secondaryHighlightColor
                                                  : Theme.secondaryColor
                    textFormat: Text.PlainText
                    text: model.topic
                }

                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeTiny
                    color: resultItem.highlighted ? Theme.secondaryHighlightColor
                                                  : Theme.secondaryColor
                    textFormat: Text.PlainText
                    text: (model.alias.length > 0 ? model.alias + " · " : "")
                          + qsTr("%n member(s)", "", model.members)
                }
            }

            menu: ContextMenu {
                MenuItem {
                    text: model.canJoin ? qsTr("Join") : qsTr("Join (invitation required)")
                    enabled: model.canJoin
                    onClicked: {
                        var target = model.alias.length > 0 ? model.alias : model.id
                        page.activeRemorse = resultItem.remorseAction(qsTr("Joining"), function() {
                            // Leaving the page has to abort the countdown.
                            // Silica's default is the opposite: it executes
                            // the action on PageStatus.Deactivating.
                            if (page.status !== PageStatus.Active || !Qt.application.active) {
                                return
                            }
                            matrix.joinRoomByAlias(target)
                        })
                    }
                }
            }
        }

        // The server pages its answer; scrolling to the end asks for more.
        onAtYEndChanged: {
            if (atYEnd && count > 0 && !matrix.directoryAtEnd) {
                matrix.directoryLoadMore()
            }
        }

        ViewPlaceholder {
            enabled: resultList.count === 0
            text: qsTr("No rooms found")
            hintText: qsTr("Public rooms of the chosen directory show up here.")
        }

        VerticalScrollDecorator { }
    }
}
