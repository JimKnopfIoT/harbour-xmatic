import QtQuick 2.0
import Sailfish.Silica 1.0

// The space overview: the joined spaces, each a folder of rooms. Tapping one
// opens its rooms on their own page (SpacePage). Order, names and unread
// counts come from the sync service in the core, exactly like the chat list;
// this is the same room list under a "spaces only" filter.
Page {
    id: page

    // Set when this is the start page: it then attaches the room list so a
    // sideways swipe reaches it, instead of going through the menu.
    property bool isHome: false

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

    Component.onCompleted: matrix.startSpaces()
    Component.onDestruction: matrix.stopSpaces()

    onStatusChanged: {
        if (status === PageStatus.Active) {
            // Returning here from a space (or a nested space) means no space is
            // open any more; opening one always replaces it, so this is the one
            // place that closes it.
            matrix.closeSpace()
            // Re-attach the room list when this is the home page, so a sideways
            // swipe still reaches it after a space was opened and closed.
            if (isHome && !pageStack.nextPage(page)) {
                pageStack.pushAttached(Qt.resolvedUrl("RoomListPage.qml"), { isHome: false })
            }
        }
    }

    // Asks first and names the space, then runs the remorse as the undo. The
    // remorse hangs on the page rather than on the row — a RemorseItem in a
    // delegate fires when that delegate is destroyed, and this list re-sorts
    // on its own — and it only acts while the page is still the active one:
    // Silica otherwise completes a running countdown on
    // PageStatus.Deactivating, which turns swiping away into a confirmation.
    function confirmDelete(spaceId, spaceName) {
        var dialog = pageStack.push(
                    Qt.resolvedUrl("ConfirmDialog.qml"),
                    {
                        question: qsTr("Really delete this space?"),
                        subject: spaceName,
                        explanation: qsTr("The space is left and forgotten. The rooms in it are not touched — they stay in the chat list."),
                        acceptLabel: qsTr("Delete")
                    })
        dialog.accepted.connect(function() {
            page.activeRemorse = Remorse.popupAction(page, qsTr("Deleting space"), function() {
                if (page.status !== PageStatus.Active || !Qt.application.active) {
                    return
                }
                matrix.leaveSpace(spaceId)
            })
        })
    }

    SilicaListView {
        id: spaceList

        anchors.fill: parent
        model: matrix.spaces

        header: PageHeader {
            title: qsTr("Spaces")
            // The core reconnects on its own; this only says that it is
            // doing so instead of leaving a silently stale list.
            description: matrix.syncState === "offline"
                         ? qsTr("Offline — waiting for the network") : ""
        }

        PullDownMenu {
            MenuItem {
                text: qsTr("Create space")
                onClicked: pageStack.push(Qt.resolvedUrl("CreateSpaceDialog.qml"))
            }

            MenuItem {
                // Only shown when spaces is not already the start page; picking
                // it takes effect on the next start.
                text: qsTr("Make start page")
                visible: settings.startPage !== "spaces"
                onClicked: settings.startPage = "spaces"
            }
        }

        delegate: RoomDelegate {
            id: spaceItem

            // Referencing spaceCounts makes the badge re-evaluate whenever a
            // child's unread count or a space's structure changes.
            trailingText: (matrix.spaceCounts, matrix.spaceBadge(model.id))
            onClicked: pageStack.push(Qt.resolvedUrl("SpacePage.qml"), {
                                          spaceId: model.id,
                                          spaceName: model.name
                                      })

            menu: ContextMenu {
                MenuItem {
                    // Leaves and forgets the space. The rooms inside it are
                    // not touched — they stay in the chat list.
                    text: qsTr("Delete space")
                    onClicked: page.confirmDelete(model.id, model.name)
                }
            }
        }

        ViewPlaceholder {
            enabled: spaceList.count === 0
            text: qsTr("No spaces")
            hintText: qsTr("Spaces you are a member of show up here. Spaces group rooms together.")
        }

        VerticalScrollDecorator { }
    }
}
