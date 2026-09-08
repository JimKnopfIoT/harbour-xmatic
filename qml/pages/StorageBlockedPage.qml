import QtQuick 2.0
import Sailfish.Silica 1.0

// Nothing on this device yet and no key to be had, so no store is created -
// the one moment at which an unencrypted one would come into existence.
Page {
    id: page

    allowedOrientations: Orientation.All

    /// Set once a check has run and found nothing. Only then is there anything
    /// to say - before that the page has not made a claim.
    property bool checked: false

    /// A check of this page's own is under way. `obtainStoreKey` blocks, so this
    /// is short - but the button must not hang on anything else in the app.
    property bool checking: false

    Connections {
        target: matrix
        onStoreKeyChecked: {
            // A check that succeeded takes this page away with it; there is
            // nothing to report on a page that is about to disappear.
            page.checking = false
            page.checked = !available
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingLarge

            PageHeader {
                title: qsTr("Encryption not possible")
                description: qsTr("Matrix for Sailfish OS")
            }

            Row {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                spacing: Theme.paddingMedium

                SecurityLamp {
                    level: "red"
                    anchors.verticalCenter: parent.verticalCenter
                }

                Label {
                    width: parent.width - Theme.iconSizeExtraSmall / 2 - Theme.paddingMedium
                    wrapMode: Text.Wrap
                    color: Theme.highlightColor
                    text: matrix.secretsDaemonPresent
                          ? qsTr("The secure storage did not hand out a key, so xmatic cannot create an encrypted database.")
                          : qsTr("This system is missing the service that keeps encryption keys, so xmatic cannot create an encrypted database.")
                }
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Check again")
                // Never on the global `busy`: this page has exactly one action,
                // and any unrelated command in flight used to disable it.
                enabled: !page.checking
                onClicked: {
                    page.checking = true
                    matrix.retryStoreKey()
                }
            }

            // The instructions live one page further in, behind the measurement.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                label: qsTr("Need help?")
                onClicked: pageStack.push(Qt.resolvedUrl("SecretsHelpPage.qml"))
            }

            // A check that finds nothing leaves the page as it was, and a button that
            // visibly does nothing reads as broken. So the check says that it ran.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                visible: page.checked
                text: qsTr("Checked — the secure storage still will not hand out a key.")
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: page.checking
            }

            // No "continue without encryption": where the service is genuinely absent the
            // package does not install, so the button only downgraded capable devices.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Messages stay end-to-end encrypted on their way through the network either way. This is only about what lies on the device.")
            }
        }

        VerticalScrollDecorator { }
    }
}
