import QtQuick 2.0
import Sailfish.Silica 1.0
import "Emoji.js" as Emoji

// Picking a reaction. Matrix carries it as a string, so these are characters,
// not images - a picture is only what the user put in the data directory.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    property string eventId
    /// The reaction the user settled on, read by whoever pushed this.
    property string key: ""

    /// Index into Emoji.groups. Group 0 is the user's own and lives in the
    /// settings, not in Emoji.js.
    property int group: 0

    /// Emoji.js only seeds the first group: once the user changed anything the
    /// setting decides, empty included - or the last removal would look undone.
    property var favourites: []

    /// Raised when one was taken in, so the group's own tab can say so - the
    /// emoji lands in a tab the user is not looking at.
    signal favouriteAdded()

    function reloadFavourites() {
        favourites = settings.hasEmojiFavourites()
                ? settings.emojiFavourites : Emoji.groups[0].items
    }

    function addFavourite(key) {
        var list = favourites.slice()
        if (list.indexOf(key) >= 0) {
            return
        }
        list.push(key)
        settings.emojiFavourites = list
        reloadFavourites()
        dialog.favouriteAdded()
    }

    function removeFavourite(key) {
        var list = []
        for (var i = 0; i < favourites.length; i++) {
            if (favourites[i] !== key) {
                list.push(favourites[i])
            }
        }
        settings.emojiFavourites = list
        reloadFavourites()
    }

    Component.onCompleted: reloadFavourites()

    /// Picking for the message being written. The accept text is then Silica's
    /// own, which is already translated everywhere.
    property bool forMessage: false

    canAccept: key.length > 0

    Column {
        id: head

        anchors { left: parent.left; right: parent.right; top: parent.top }

        DialogHeader {
            acceptText: dialog.forMessage ? defaultAcceptText : qsTr("React")
        }

        TextField {
            width: parent.width
            // The same dialog serves two jobs, and "something else" named
            // neither: here the text is reacted with, there it is typed into
            // the message.
            label: dialog.forMessage ? qsTr("Your own text")
                                     : qsTr("Your text as reaction")
            placeholderText: label
            inputMethodHints: Qt.ImhNoPredictiveText
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: {
                if (text.trim().length > 0) {
                    dialog.key = text.trim()
                    dialog.accept()
                }
            }
            onTextChanged: dialog.key = text.trim()
        }

        // One row of groups, scrolling sideways because ten do not fit. No labels: a
        // group's own emoji says what it is in every language.
        SilicaListView {
            id: groups

            width: parent.width
            height: Theme.itemSizeSmall
            orientation: ListView.Horizontal
            currentIndex: -1
            model: Emoji.groups.length
            clip: true

            delegate: BackgroundItem {
                width: Theme.itemSizeSmall
                height: groups.height
                highlighted: down || dialog.group === index

                onClicked: {
                    dialog.group = index
                    grid.positionViewAtBeginning()
                }

                EmojiItem {
                    id: groupIcon

                    anchors.centerIn: parent
                    character: Emoji.groups[index].icon
                    size: Theme.iconSizeSmall
                }

                // An emoji taken in lands in a tab that is not on screen, so the tab answers
                // for it - a word would need a place and a translation for half a second.
                SequentialAnimation {
                    id: taken

                    NumberAnimation {
                        target: groupIcon
                        property: "scale"
                        to: 1.5
                        duration: 120
                    }
                    NumberAnimation {
                        target: groupIcon
                        property: "scale"
                        to: 1.0
                        duration: 200
                    }
                }

                Connections {
                    target: dialog
                    onFavouriteAdded: {
                        if (index === 0) {
                            taken.restart()
                        }
                    }
                }
            }
        }
    }

    SilicaGridView {
        id: grid

        anchors {
            left: parent.left
            right: parent.right
            top: head.bottom
            bottom: parent.bottom
        }
        clip: true
        // Qt gives row 0 the focus unless this says otherwise, and that
        // focus is taken from the field above on every group change.
        currentIndex: -1

        readonly property int columns: Math.max(4, Math.floor(width / Theme.itemSizeSmall))

        cellWidth: Math.floor(width / columns)
        cellHeight: cellWidth
        model: dialog.group === 0 ? dialog.favourites : Emoji.groups[dialog.group].items

        delegate: BackgroundItem {
            width: grid.cellWidth
            height: grid.cellHeight
            highlighted: down || dialog.key === modelData

            onClicked: {
                dialog.key = modelData
                dialog.accept()
            }

            // A long press takes one into the first group and back out. Taken in, not
            // moved: the same character can be in two groups.
            onPressAndHold: {
                if (dialog.group === 0) {
                    dialog.removeFavourite(modelData)
                } else {
                    dialog.addFavourite(modelData)
                }
            }

            EmojiItem {
                anchors.centerIn: parent
                character: modelData
                size: Theme.iconSizeMedium
            }
        }

        // Only the user's own group can be empty, and only because the user
        // emptied it.
        ViewPlaceholder {
            enabled: dialog.group === 0 && dialog.favourites.length === 0
            text: qsTr("Nothing kept here")
            hintText: qsTr("Press and hold an emoji in another tab to keep it here")
        }

        VerticalScrollDecorator {}
    }
}
