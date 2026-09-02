import QtQuick 2.0
import Sailfish.Silica 1.0

// Sign-in. The server authenticates in its own web UI, so this page takes the
// homeserver and hands over to the browser; the core waits for the redirect.
Page {
    id: page

    allowedOrientations: Orientation.All

    // Set while a device-code login is waiting for approval elsewhere.
    property string deviceUrl
    property string deviceCode

    // Set only by the core, from its own discovery - never as a fallback for a
    // failed OAuth attempt, so a broken auth service cannot lure a password here.
    property bool passwordLogin

    // The password must not outlive its moment: leaving the page or going to the
    // background clears the field.
    onStatusChanged: {
        if (status !== PageStatus.Active) {
            passwordField.text = ""
        }
    }

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
                enabled: !matrix.loginRunning
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.signIn()
                // A different server means fresh discovery. The null check matters: this
                // fires for the initial text, while the fields below do not exist yet.
                onTextChanged: {
                    page.passwordLogin = false
                    if (passwordField) {
                        passwordField.text = ""
                    }
                }
            }

            TextField {
                id: userField

                width: parent.width
                visible: page.passwordLogin
                label: qsTr("Username")
                placeholderText: qsTr("Username")
                inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase | Qt.ImhPreferLowercase
                enabled: !matrix.loginRunning
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: passwordField.focus = true
            }

            PasswordField {
                id: passwordField

                width: parent.width
                visible: page.passwordLogin
                label: qsTr("Password")
                placeholderText: qsTr("Password")
                // Out of the keyboard's prediction database in any case;
                // PasswordField hides the echo, this keeps the storage clean.
                inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhSensitiveData | Qt.ImhNoAutoUppercase
                enabled: !matrix.loginRunning
                EnterKey.iconSource: "image://theme/icon-m-enter-accept"
                EnterKey.onClicked: page.signInWithPassword()
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.passwordLogin
                label: qsTr("Sign in")
                enabled: !matrix.loginRunning && userField.text.trim().length > 0
                         && passwordField.text.length > 0
                onClicked: page.signInWithPassword()
            }

            // The ordinary way in, and therefore first. It stays even where the browser
            // cannot finish - a server may not need it - and the note points elsewhere.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: !page.passwordLogin
                label: matrix.browserLoginReliable ? qsTr("Sign in")
                                                  : qsTr("Sign in via browser")
                enabled: !matrix.loginRunning && homeserverField.text.trim().length > 0
                onClicked: page.signIn()
            }

            // Directly above the button it names, and only where the browser is the one
            // that cannot finish: 4.6's Gecko loops back to the form.
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.errorColor
                visible: !page.passwordLogin && !matrix.browserLoginReliable
                         && !matrix.loginRunning
                text: qsTr("This Sailfish version's browser cannot complete the sign-in of modern homeservers — it returns to the form. Use “Sign in on another device”: xmatic shows an address and a code, you sign in with them on any other device, and this one signs in by itself.")
            }

            // The second way in, and on a release whose browser is the
            // problem the only one that works.
            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: !page.passwordLogin
                label: qsTr("Sign in on another device")
                enabled: !matrix.loginRunning && homeserverField.text.trim().length > 0
                onClicked: {
                    homeserverField.focus = false
                    matrix.startDeviceCodeLogin(homeserverField.text)
                }
            }

            WrapButton {
                anchors.horizontalCenter: parent.horizontalCenter
                // Classic servers have no registration page to point at and no device-code
                // grant: both entries only make sense before the server says "password".
                visible: !page.passwordLogin
                label: qsTr("Create account")
                enabled: !matrix.loginRunning && homeserverField.text.trim().length > 0
                onClicked: matrix.requestRegistrationUrl(homeserverField.text)
            }


            BusyIndicator {
                anchors.horizontalCenter: parent.horizontalCenter
                size: BusyIndicatorSize.Medium
                running: matrix.loginRunning
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                visible: matrix.loginRunning && page.deviceCode.length === 0 && !page.passwordLogin
                text: qsTr("Finish signing in in the browser, then come back.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                visible: matrix.loginRunning && page.deviceCode.length > 0
                text: qsTr("Open this address on any other device and sign in there:")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.WrapAnywhere
                horizontalAlignment: Text.AlignHCenter
                color: Theme.highlightColor
                visible: matrix.loginRunning && page.deviceCode.length > 0
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
                visible: matrix.loginRunning && page.deviceCode.length > 0
                text: page.deviceCode
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryColor
                visible: matrix.loginRunning && page.deviceCode.length > 0
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
                // True for the OAuth flows and only for them; the password
                // state has its own, honest line below.
                visible: !page.passwordLogin
                text: qsTr("Your password is entered on the homeserver's own page and never reaches this app.")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryColor
                visible: page.passwordLogin
                text: qsTr("This server uses the classic password sign-in. The password is sent only to this server and is never saved on the device.")
            }

            // The failure is named: a wrong password, an unreachable server and an
            // unimplemented method arrived as one red line, and only the first is retypable.
            Column {
                width: parent.width
                spacing: Theme.paddingSmall
                visible: matrix.lastError.length > 0

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.errorColor
                    textFormat: Text.PlainText
                    text: qsTr("Sign-in did not work")
                }

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryColor
                    textFormat: Text.PlainText
                    text: matrix.lastError
                }

                // What to do about it, where the message allows a guess.
                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryHighlightColor
                    textFormat: Text.PlainText
                    visible: text.length > 0
                    text: {
                        var message = matrix.lastError
                        if (message.indexOf("SSO") >= 0) {
                            return qsTr("This is the server's own web sign-in. Your password is not wrong — the app cannot use this method yet.")
                        }
                        if (message.indexOf("unreachable") >= 0) {
                            return qsTr("Check the server address and your connection.")
                        }
                        if (message.indexOf("no sign-in method") >= 0) {
                            return qsTr("The server expects a sign-in this app does not implement.")
                        }
                        return ""
                    }
                }
            }
        }

        PullDownMenu {
            MenuItem {
                text: qsTr("Cancel sign-in")
                visible: matrix.loginRunning
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
        // Registration happens on the server's own page - terms, captcha, e-mail
        // confirmation, none of it worth reimplementing.
        onRegistrationUrlReady: Qt.openUrlExternally(url)
        onDeviceCodeReady: {
            page.deviceUrl = url
            page.deviceCode = code
        }
        // A failed login clears the code, or the stale block reappears with the next
        // busy. The password form stays: a wrong password is retyped into it.
        onLoginFailed: {
            page.deviceUrl = ""
            page.deviceCode = ""
        }
        onPasswordLoginNeeded: {
            page.passwordLogin = true
            userField.focus = true
        }
    }

    Connections {
        target: Qt.application
        // Another window over this one — task switcher, another app pushing
        // itself in front — must not inherit a half-typed password.
        onActiveChanged: {
            if (!Qt.application.active) {
                passwordField.text = ""
            }
        }
    }

    function signIn() {
        homeserverField.focus = false
        matrix.startLogin(homeserverField.text)
    }

    function signInWithPassword() {
        // The field is cleared before the command goes out; the local copy
        // lives exactly as long as this call.
        var password = passwordField.text
        passwordField.text = ""
        // Same rule as the recovery key: a pasted password stays in the clipboard,
        // and only this exact text is taken out of it.
        if (Clipboard.hasText && Clipboard.text === password) {
            Clipboard.text = ""
        }
        passwordField.focus = false
        homeserverField.focus = false
        matrix.startPasswordLogin(homeserverField.text, userField.text, password)
        password = ""
    }
}
