import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailfish.Gallery 1.0
import QtDocGallery 5.0
import Qt.labs.folderlistmodel 2.1

// Two ways to the same thing, side by side: the gallery, divided by folder,
// and the file system from the home folder down. The platform offers each of
// them but not together, and its gallery is one undivided grid.
Dialog {
    id: dialog

    allowedOrientations: Orientation.All

    /// What the caller reads once accepted: [{ path, mimeType }, …] in the
    /// order they were tapped.
    property var picked: []

    /// Set instead of a file where the pull-down asks for the platform's own
    /// picker: a dialog cannot push a page and pop itself in one breath.
    property bool otherFilesWanted: false

    /// "gallery" or "files".
    property string mode: "gallery"

    /// The gallery's folder. Empty is every picture below the home folder.
    property string folder: ""

    // The home folder is start and ceiling: below it are the only directories
    // the sandbox shows, above it nothing worth walking into.
    readonly property string root: StandardPaths.home
    property string directory: StandardPaths.home

    readonly property bool canGoUp: dialog.directory !== dialog.root
                                    && dialog.directory.length > dialog.root.length

    canAccept: picked.length > 0 || otherFilesWanted

    function up() {
        if (!dialog.canGoUp) {
            return
        }
        var cut = dialog.directory.lastIndexOf("/")
        dialog.directory = cut > dialog.root.length
                           ? dialog.directory.substring(0, cut) : dialog.root
    }

    function indexOfPath(path) {
        for (var i = 0; i < dialog.picked.length; i++) {
            if (dialog.picked[i].path === path) {
                return i
            }
        }
        return -1
    }

    /// Adds or removes one file. The whole array is replaced rather than
    /// changed in place: a binding follows the property, not its contents.
    function toggle(path, mimeType) {
        var next = dialog.picked.slice()
        var at = dialog.indexOfPath(path)
        if (at >= 0) {
            next.splice(at, 1)
        } else {
            next.push({
                "path": path,
                "mimeType": mimeType && mimeType.length > 0
                            ? mimeType : matrix.mimeTypeForPath(path)
            })
        }
        dialog.picked = next
    }

    function askForOtherFiles() {
        dialog.otherFilesWanted = true
        dialog.accept()
    }

    // The folders that exist, not the ones we imagine: the strip is built from
    // what lies under Pictures, plus the two places that are always there.
    ListModel {
        id: sources
    }

    FolderListModel {
        id: pictureFolders

        folder: "file://" + StandardPaths.pictures
        showDirs: true
        showFiles: false
        showDotAndDotDot: false
        sortField: FolderListModel.Name

        onCountChanged: dialog.rebuildSources()
    }

    function rebuildSources() {
        var shown = dialog.folder
        sources.clear()
        sources.append({ "name": qsTr("All"), "path": "" })
        sources.append({ "name": qsTr("Pictures"), "path": StandardPaths.pictures })
        for (var i = 0; i < pictureFolders.count; i++) {
            sources.append({
                "name": String(pictureFolders.get(i, "fileName")),
                "path": String(pictureFolders.get(i, "filePath"))
            })
        }
        sources.append({ "name": qsTr("Downloads"), "path": StandardPaths.download })
        dialog.folder = shown
    }

    Component.onCompleted: rebuildSources()

    DocumentGalleryModel {
        id: pictures

        rootType: DocumentGallery.Image
        properties: ["url", "filePath", "fileName", "mimeType", "lastModified"]
        sortProperties: ["-lastModified"]
        autoUpdate: true

        filter: GalleryStartsWithFilter {
            property: "filePath"
            value: dialog.folder.length > 0 ? dialog.folder : StandardPaths.home
        }
    }

    // Outside both views: whichever one is hidden must not take the way back
    // with it. Every state keeps a visible action.
    Column {
        id: head

        anchors { left: parent.left; right: parent.right; top: parent.top }

        DialogHeader {
            acceptText: dialog.picked.length > 0
                        ? qsTr("%1 selected").arg(dialog.picked.length)
                        : qsTr("Attachment")
        }

        Row {
            width: parent.width

            ModeTab {
                width: parent.width / 2
                label: qsTr("Gallery")
                active: dialog.mode === "gallery"
                onClicked: dialog.mode = "gallery"
            }

            ModeTab {
                width: parent.width / 2
                label: qsTr("Files")
                active: dialog.mode === "files"
                onClicked: dialog.mode = "files"
            }
        }

        // Sideways, because the folder names are as long as they are and a
        // menu of them could not be scrolled in landscape.
        SilicaListView {
            id: strip

            width: parent.width
            height: dialog.mode === "gallery" ? Theme.itemSizeSmall : 0
            visible: dialog.mode === "gallery"
            orientation: ListView.Horizontal
            currentIndex: -1
            clip: true
            model: sources

            delegate: BackgroundItem {
                width: sourceName.implicitWidth + 2 * Theme.paddingLarge
                height: strip.height

                onClicked: dialog.folder = model.path

                Label {
                    id: sourceName

                    anchors.centerIn: parent
                    font.pixelSize: Theme.fontSizeSmall
                    color: dialog.folder === model.path ? Theme.highlightColor
                                                        : Theme.primaryColor
                    text: model.name
                }
            }
        }

        // Where the browser stands, and the way up. A row rather than a menu
        // entry: it is the only way back out of a folder.
        BackgroundItem {
            width: parent.width
            height: dialog.mode === "files" ? Theme.itemSizeSmall : 0
            visible: dialog.mode === "files"
            enabled: dialog.canGoUp

            onClicked: dialog.up()

            Image {
                id: upMark

                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                visible: dialog.canGoUp
                source: "image://theme/icon-m-back"
            }

            Label {
                anchors {
                    left: upMark.right
                    leftMargin: Theme.paddingMedium
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                truncationMode: TruncationMode.Fade
                text: dialog.directory === dialog.root
                      ? qsTr("Home folder")
                      : dialog.directory.substring(dialog.root.length + 1)
            }
        }
    }

    SilicaGridView {
        id: grid

        readonly property int columns: Math.max(1, Math.floor(width / Theme.itemSizeHuge))

        anchors {
            left: parent.left
            right: parent.right
            top: head.bottom
            bottom: parent.bottom
        }
        visible: dialog.mode === "gallery"
        // Qt Quick focuses row 0 on every model reset; switching folders is one.
        currentIndex: -1
        cellWidth: Math.floor(width / columns)
        cellHeight: cellWidth
        model: pictures
        clip: true

        PullDownMenu {
            MenuItem {
                text: qsTr("Other files")
                onClicked: dialog.askForOtherFiles()
            }
        }

        delegate: ThumbnailImage {
            source: model.url
            size: grid.cellWidth
            // The platform's own selection look comes with the component.
            selected: dialog.indexOfPath(model.filePath) >= 0
            onClicked: dialog.toggle(model.filePath, model.mimeType)
        }

        ViewPlaceholder {
            enabled: pictures.count === 0
            text: qsTr("No pictures here")
        }

        VerticalScrollDecorator { }
    }

    SilicaListView {
        id: files

        anchors {
            left: parent.left
            right: parent.right
            top: head.bottom
            bottom: parent.bottom
        }
        visible: dialog.mode === "files"
        currentIndex: -1
        clip: true

        PullDownMenu {
            MenuItem {
                text: qsTr("Other files")
                onClicked: dialog.askForOtherFiles()
            }
        }

        model: FolderListModel {
            id: entries

            folder: "file://" + dialog.directory
            showDirs: true
            showDirsFirst: true
            showFiles: true
            showDotAndDotDot: false
            showHidden: false
            sortField: FolderListModel.Name
        }

        delegate: ListItem {
            id: row

            width: files.width
            highlighted: down || dialog.indexOfPath(model.filePath) >= 0

            // A picture shows itself; everything else gets the theme's mark.
            // Asked for at the size it is drawn, not at the size it was taken.
            Image {
                id: mark

                anchors {
                    left: parent.left
                    leftMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                width: Theme.iconSizeMedium
                height: Theme.iconSizeMedium
                fillMode: Image.PreserveAspectCrop
                clip: true
                asynchronous: true
                sourceSize.width: Theme.iconSizeMedium
                sourceSize.height: Theme.iconSizeMedium
                source: {
                    if (model.fileIsDir) {
                        return "image://theme/icon-m-file-folder"
                    }
                    var kind = matrix.mimeTypeForPath(model.filePath)
                    if (kind.indexOf("image/") === 0) {
                        return "file://" + model.filePath
                    }
                    if (kind.indexOf("video/") === 0) {
                        return "image://theme/icon-m-file-video"
                    }
                    if (kind.indexOf("audio/") === 0) {
                        return "image://theme/icon-m-file-audio"
                    }
                    return "image://theme/icon-m-file-other"
                }
            }

            Label {
                anchors {
                    left: mark.right
                    leftMargin: Theme.paddingMedium
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                text: model.fileName
                truncationMode: TruncationMode.Fade
            }

            onClicked: {
                if (model.fileIsDir) {
                    dialog.directory = model.filePath
                    return
                }
                dialog.toggle(model.filePath, "")
            }
        }

        ViewPlaceholder {
            enabled: entries.count === 0
            text: qsTr("Nothing here")
        }

        VerticalScrollDecorator { }
    }
}
