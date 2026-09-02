import QtQuick 2.0
import Sailfish.Silica 1.0
import "SecurityStatus.js" as SecurityStatus

// What the marks next to a message mean. The wording is shared with the
// sentence a tapped mark shows: a mark without a way to look it up is worse.
Page {
    id: page

    allowedOrientations: Orientation.All

    readonly property color redColor: Theme.errorColor
    readonly property color greyColor:
        SecurityStatus.color(SecurityStatus.ORANGE, Theme,
                             Theme.colorScheme === Theme.LightOnDark)

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: content.height

        Column {
            id: content

            width: page.width
            spacing: Theme.paddingLarge

            PageHeader {
                title: qsTr("Message marks")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryColor
                text: qsTr("A mark beside a message means its authenticity could not be fully confirmed. A red triangle says the message is not what it claims to be. An orange dot says something about it could not be checked. Tap a mark in the conversation to see which of the cases below it is.")
            }

            Repeater {
                model: [
                    { red: true,  name: qsTr("Sent unencrypted"),
                      text: qsTr("The message went out without encryption although the room uses it. Anybody who can read the server's copy can read the message.") },
                    { red: true,  name: qsTr("Not sent by the account it names"),
                      text: qsTr("The message was encrypted by a device that does not belong to the account it claims to come from - somebody is sending under another name. Some bridges work this way and produce it harmlessly; anywhere else it is the one sign of an actual impersonation this app has.") },
                    { red: true,  name: qsTr("The sender's keys changed"),
                      text: qsTr("This person's cryptographic identity is no longer the one that was verified. Either they set up their account again, or somebody else is using it.") },
                    { red: false, name: qsTr("From an unverified device"),
                      text: qsTr("The device that sent this has not been confirmed by the account it belongs to. That is normal for a device somebody has just started using.") },
                    { red: false, name: qsTr("From an unverified person"),
                      text: qsTr("This person's identity has never been verified, so there is nothing to check the message against.") },
                    { red: false, name: qsTr("Authenticity not confirmed"),
                      text: qsTr("The message could not be checked at all. It is not necessarily wrong - only unproven.") }
                ]

                Column {
                    x: Theme.horizontalPageMargin
                    width: content.width - 2 * Theme.horizontalPageMargin
                    spacing: Theme.paddingSmall

                    Row {
                        spacing: Theme.paddingMedium

                        Label {
                            anchors.verticalCenter: parent.verticalCenter
                            text: modelData.red ? "▲" : "●"
                            color: modelData.red ? page.redColor : page.greyColor
                        }

                        Label {
                            anchors.verticalCenter: parent.verticalCenter
                            text: modelData.name
                            color: modelData.red ? page.redColor : Theme.primaryColor
                        }
                    }

                    Label {
                        width: parent.width
                        text: modelData.text
                        wrapMode: Text.Wrap
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.secondaryColor
                    }
                }
            }

            // Air under the last entry, as its own item: QtQuick 2.0 knows no padding on a
            // positioner, and `bottomPadding` keeps the page from loading at all.
            Item {
                width: 1
                height: Theme.paddingLarge
            }
        }

        VerticalScrollDecorator { }
    }
}
