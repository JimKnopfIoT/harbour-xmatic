import QtQuick 2.0
import Sailfish.Silica 1.0

// Sign-in. matrix.org authenticates through its own web UI, so all this page
// does is take the homeserver and hand the user over to the browser; the core
// waits for the redirect on a loopback listener and reports back.
Page {
    id: page

    allowedOrientations: Orientation.All

    // Set while a device-code login is waiting for approval elsewhere.
    property string deviceUrl
    property string deviceCode

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column

            width: page.width
            spacing: Theme.paddingMedium

            PageHeader {
                title: qsTr("Sign in")
                description: qsTr("Matrix for Sailfish OS")
            }

            TextField {
                id: homeserverField

                width: parent.width
                label: qsTr("Homeserver")
                placeholderText: qsTr("Homeserver")
                text: "matrix.org"
                inputMethodHints: Qt.ImhUrlCharactersOnly | Qt.ImhNoAutoUppercase
                enabled: !matrix.busy
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.signIn()
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Sign in")
                enabled: !matrix.busy && homeserverField.text.trim().length > 0
                onClicked: page.signIn()
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Create account")
                enabled: !matrix.busy && homeserverField.text.trim().length > 0
                onClicked: matrix.requestRegistrationUrl(homeserverField.text)
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                // The way in when this device's own browser cannot handle the
                // server's sign-in pages, as on Sailfish 4.6.
                text: qsTr("Sign in on another device")
                enabled: !matrix.busy && homeserverField.text.trim().length > 0
                onClicked: {
                    homeserverField.focus = false
                    matrix.startDeviceCodeLogin(homeserverField.text)
                }
            }

            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: matrix.busy
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                visible: matrix.busy && page.deviceCode.length === 0
                text: qsTr("Finish signing in in the browser, then come back.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                visible: matrix.busy && page.deviceCode.length > 0
                text: qsTr("Open this address on any other device and sign in there:")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.WrapAnywhere
                horizontalAlignment: Text.AlignHCenter
                color: Theme.highlightColor
                visible: matrix.busy && page.deviceCode.length > 0
                text: page.deviceUrl
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.WrapAnywhere
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeExtraLarge
                font.letterSpacing: 2
                color: Theme.highlightColor
                visible: matrix.busy && page.deviceCode.length > 0
                text: page.deviceCode
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryColor
                visible: matrix.busy && page.deviceCode.length > 0
                text: qsTr("This page signs in by itself as soon as the login is approved there.")
            }

            Item {
                width: 1
                height: Theme.paddingLarge
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                text: qsTr("Your password is entered on the homeserver's own page and never reaches this app.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                visible: matrix.lastError.length > 0
                text: matrix.lastError
            }
        }

        PullDownMenu {
            MenuItem {
                text: qsTr("Cancel sign-in")
                visible: matrix.busy
                onClicked: {
                    page.deviceUrl = ""
                    page.deviceCode = ""
                    matrix.abortLogin()
                }
            }
        }

        VerticalScrollDecorator { }
    }

    Connections {
        target: matrix
        // Registration happens on the server's own page: it involves terms, a
        // captcha and e-mail confirmation, none of which are worth
        // reimplementing here.
        onRegistrationUrlReady: Qt.openUrlExternally(url)
        onDeviceCodeReady: {
            page.deviceUrl = url
            page.deviceCode = code
        }
        // A failed or aborted login must clear the code, or the stale block
        // reappears the next time something sets busy.
        onLoginFailed: {
            page.deviceUrl = ""
            page.deviceCode = ""
        }
    }

    function signIn() {
        homeserverField.focus = false
        matrix.startLogin(homeserverField.text)
    }
}
