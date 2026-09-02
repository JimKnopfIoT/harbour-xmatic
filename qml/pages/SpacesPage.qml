import QtQuick 2.0
import Sailfish.Silica 1.0

import "SecurityStatus.js" as SecurityStatus

// The space overview: joined spaces, each a folder of rooms. The same room
// list under a "spaces only" filter, so order and counts come from the core.
Page {
    id: page

    // Set when this is the start page: it then attaches the room list so a
    // sideways swipe reaches it, instead of going through the menu.
    property bool isHome: false

    // Leaving the foreground aborts a running countdown at once: suppressing it
    // at expiry left it firing on return from a minimised app.
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
            // Coming back here means no space is open. Opening one always replaces it, so
            // this is the one place that closes it.
            matrix.closeSpace()
            // Re-attach the room list when this is the home page, so a sideways
            // swipe still reaches it after a space was opened and closed.
            if (isHome && !pageStack.nextPage(page)) {
                pageStack.pushAttached(Qt.resolvedUrl("RoomListPage.qml"), { isHome: false })
            }
        }
    }

    // Asks first, then the remorse as the undo. On the page, not the row - a
    // delegate destroyed by a re-sort would fire it - and only while the page is up.
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

            // Same indicator as on the chat list; a user who starts here must
            // not have to know that the other page carries it.
            MouseArea {
                anchors.left: parent.left
                anchors.leftMargin: Theme.horizontalPageMargin
                anchors.verticalCenter: parent.verticalCenter
                width: Theme.itemSizeExtraSmall
                height: width
                visible: SecurityStatus.needsAttention(matrix)
                onClicked: pageStack.push(Qt.resolvedUrl("SecurityStatusPage.qml"))

                SecurityLamp {
                    anchors.centerIn: parent
                    overall: true
                }
            }
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
