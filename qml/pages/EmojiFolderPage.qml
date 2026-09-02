import QtQuick 2.0
import Sailfish.Silica 1.0
import Qt.labs.folderlistmodel 2.1

// Our own folder browser. The platform's picker opens on a partition list and
// takes a folder through an accept action whose label is a blank space.
Page {
    id: page

    allowedOrientations: Orientation.All

    // The home folder is the ceiling as well as the start: below it are the only
    // directories the sandbox shows anyway, above it nothing worth walking into.
    readonly property string root: StandardPaths.home
    property string folder: page.root

    readonly property bool canGoUp: page.folder !== page.root
                                    && page.folder.length > page.root.length

    function up() {
        if (!page.canGoUp) {
            return
        }
        var cut = page.folder.lastIndexOf("/")
        page.folder = cut > page.root.length ? page.folder.substring(0, cut)
                                             : page.root
    }

    SilicaListView {
        id: view

        anchors.fill: parent
        // Qt Quick focuses row 0 on every model reset and takes it from whatever
        // held it. Every list in this app sets this.
        currentIndex: -1

        header: PageHeader {
            title: qsTr("Where is the pack?")
            description: page.folder
        }

        PullDownMenu {
            MenuItem {
                text: qsTr("Take this folder")
                onClicked: {
                    emojiSet.importFrom(page.folder)
                    pageStack.pop()
                }
            }
            MenuItem {
                text: qsTr("One level up")
                enabled: page.canGoUp
                onClicked: page.up()
            }
        }

        model: FolderListModel {
            id: folders

            folder: "file://" + page.folder
            showFiles: false
            showDirs: true
            showDotAndDotDot: false
            showHidden: false
            sortField: FolderListModel.Name
        }

        delegate: ListItem {
            id: row

            // Looked at before the tap, so the row can say what a tap will do:
            // a folder with pictures in it is read in, any other is opened.
            readonly property bool pack: emojiSet.holdsPictures(model.filePath)

            width: view.width

            Image {
                id: mark

                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                source: "image://theme/icon-m-file-folder"
            }

            Column {
                anchors {
                    left: mark.right
                    leftMargin: Theme.paddingMedium
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }

                Label {
                    width: parent.width
                    text: model.fileName
                    truncationMode: TruncationMode.Fade
                }

                Label {
                    width: parent.width
                    visible: row.pack
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.highlightColor
                    text: qsTr("Pictures in here - tap to read them in")
                    truncationMode: TruncationMode.Fade
                }
            }

            onClicked: {
                if (row.pack) {
                    emojiSet.importFrom(model.filePath)
                    pageStack.pop()
                } else {
                    page.folder = model.filePath
                }
            }
        }

        ViewPlaceholder {
            enabled: folders.count === 0
            text: qsTr("No folders in here")
            hintText: qsTr("The pull-down takes the folder you are in.")
        }

        VerticalScrollDecorator { }
    }
}
