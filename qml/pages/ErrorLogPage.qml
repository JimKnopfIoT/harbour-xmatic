import QtQuick 2.0
import Sailfish.Silica 1.0

// Everything that failed in this run of the app.
//
// `lastError` is one string and the next failure overwrites it, so a fault
// that showed up as a red line on some page is gone the moment anything else
// goes wrong. A field report then rests on whether the user happened to have
// the right page open at the right second — which is how most of this app's
// harder bugs were reported: seen once, described from memory, gone by the
// time anybody could look.
//
// The core takes identifiers out of its messages before they leave it
// (`core/src/text.rs`, `scrub_ids`), keeping the error codes that make a
// report useful. So this list can be copied and handed over as it stands.
//
// Not written to disk. What is on screen is this run, and a log that outlived
// the run would be a file of failures nobody asked us to keep.
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

            // In the header rather than a ViewPlaceholder: Silica centres a
            // placeholder in the viewport and ignores the header, so the two
            // draw over each other on a short page.
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
            // No `bottomPadding`: QtQuick 2.0's Column has no such property and
            // assigning it makes the page refuse to load, with nothing but
            // "could not load page" to go on. A spacer at the end does the job.

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
