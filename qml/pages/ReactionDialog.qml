import QtQuick 2.0
import Sailfish.Silica 1.0
import "Emoji.js" as Emoji

// Picking a reaction for one message.
//
// A reaction is not an icon set: Matrix carries it as a string, usually one
// emoji character, and whatever another client sent has to be drawn as it
// arrived. So these are characters, not images - a picture is only what the
// user put into the app's data directory, looked up from the character.
//
// The grid is the Unicode set in Unicode's own order and groups (Emoji.js),
// the first group being the common handful. Anything the set does not have -
// a word, a rare sequence - is typed into the field, which the system
// keyboard fills.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    property string eventId
    /// The reaction the user settled on, read by whoever pushed this.
    property string key: ""

    /// Index into Emoji.groups. Group 0 is the user's own: what it holds is
    /// kept in the settings, not in Emoji.js, and a long press moves emoji in
    /// and out of it.
    property int group: 0

    /// The first group's contents. Emoji.js only seeds it: once the user has
    /// changed anything, the setting decides, empty list included - otherwise
    /// taking the last one out would put the built-in handful back and read as
    /// if the removal had failed.
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

    /// Picking for the message being written rather than for a reaction. The
    /// accept text is then Silica's own, which already says "accept" in every
    /// language the app ships - "React" would be the wrong word and a new
    /// string in twenty-nine files for a button that a tap on an emoji makes
    /// unnecessary anyway.
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
            label: qsTr("Something else")
            placeholderText: qsTr("Something else")
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

        // One row of groups. It scrolls sideways because ten of them do not
        // fit a portrait screen, and it carries no labels: a group's own
        // emoji says what it is in every language.
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

                // An emoji taken in lands in a tab that is not on screen, so
                // the tab itself answers for it. A word would need a place to
                // put it and a translation in every language for something
                // that is over in half a second.
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

            // The first group is the user's own: a long press takes one in
            // from anywhere else and takes one back out of it. Taken in, not
            // moved - it stays where it was, a group is a list of characters
            // and the same character can be in two of them.
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
