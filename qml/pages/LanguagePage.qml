import QtQuick 2.0
import Sailfish.Silica 1.0

// The UI language. A page, not a ComboBox: a Silica context menu does not
// scroll and holds about four rows in landscape, and this list has twenty-eight.
Page {
    id: page

    allowedOrientations: Orientation.All

    SilicaListView {
        anchors.fill: parent
        // Nothing here takes focus, but the list resets when the choice is
        // written; row 0 must not be selected on its own.
        currentIndex: -1
        model: language.available

        header: Column {
            width: page.width

            PageHeader {
                title: qsTr("Language")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Takes effect the next time the app starts. Only the German translation has been checked by a native speaker; the others are machine translations, and English is always available here.")
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }

        delegate: BackgroundItem {
            width: page.width

            readonly property bool isCurrent: modelData.code === language.code

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                anchors.verticalCenter: parent.verticalCenter
                truncationMode: TruncationMode.Fade
                textFormat: Text.PlainText
                color: parent.isCurrent ? Theme.highlightColor
                                        : (parent.highlighted ? Theme.highlightColor
                                                              : Theme.primaryColor)
                text: modelData.name
            }

            onClicked: {
                language.code = modelData.code
                pageStack.pop()
            }
        }

        VerticalScrollDecorator { }
    }
}
