import QtQuick 2.0
import Sailfish.Silica 1.0

// Everything that failed in this run. `lastError` is one string the next
// failure overwrites; the core scrubs identifiers, so this can be handed over.
Page {
    id: page

    allowedOrientations: Orientation.All

    function asText() {
        var lines = []
        for (var i = 0; i < matrix.errorLog.length; ++i) {
            var entry = matrix.errorLog[i]
            lines.push(entry.time + "  " + entry.command + "  " + entry.message)
        }
        return lines.join("\n")
    }

    SilicaListView {
        id: listView

        anchors.fill: parent
        model: matrix.errorLog

        PullDownMenu {
            MenuItem {
                text: qsTr("Clear")
                visible: matrix.errorLog.length > 0
                onClicked: matrix.clearErrorLog()
            }

            MenuItem {
                text: qsTr("Copy all")
                visible: matrix.errorLog.length > 0
                onClicked: Clipboard.text = page.asText()
            }
        }

        header: Column {
            width: listView.width

            PageHeader {
                title: qsTr("Error log")
                description: matrix.errorLog.length > 0
                             ? String(matrix.errorLog.length) : ""
            }

            // In the header rather than a ViewPlaceholder: Silica centres a placeholder in
            // the viewport and ignores the header, so the two draw over each other.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: matrix.errorLog.length === 0
                wrapMode: Text.Wrap
                color: Theme.secondaryColor
                text: qsTr("Nothing has failed in this run.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                visible: matrix.errorLog.length > 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Newest first, and only this run — nothing is kept on disk. Identifiers are already removed, so this can be passed on as it stands.")
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }

        delegate: Column {
            width: listView.width
            // No `bottomPadding`: QtQuick 2.0's Column has no such property and assigning
            // it makes the page refuse to load. A spacer does the job.

            Row {
                x: Theme.horizontalPageMargin
                spacing: Theme.paddingMedium

                Label {
                    font.pixelSize: Theme.fontSizeExtraSmall
                    font.family: Theme.fontFamilyHeading
                    color: Theme.highlightColor
                    text: modelData.time
                }

                Label {
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryColor
                    // Empty for the failures that never went through a command
                    // — a recording that could not start, say.
                    text: modelData.command
                }
            }

            Label {
                x: Theme.horizontalPageMargin
                width: listView.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                // The core's own words, and a stranger's server can be in
                // them: never markup.
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeSmall
                text: modelData.message
            }

            Item {
                width: 1
                height: Theme.paddingMedium
            }
        }

        VerticalScrollDecorator { }
    }
}
