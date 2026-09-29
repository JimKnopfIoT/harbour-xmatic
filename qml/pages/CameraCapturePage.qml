import QtQuick 2.5
import Sailfish.Silica 1.0
import QtMultimedia 5.6
import QtSensors 5.0

// One photo for the picker. It lands in the cache and is throwaway; a copy
// goes to the gallery only where the user says so. Camera settings are the
// device's own: nothing here asks for a resolution.
Page {
    id: page

    /// The photo, accepted. The picker takes it from here.
    signal taken(string path)

    property string shot: ""
    property bool capturing: false
    property string failure: ""
    property bool keepInGallery: false
    property bool handedOver: false
    /// The device's rotation at the shutter, as the platform camera reads it.
    property int pictureRotation: 0

    readonly property bool callRunning: matrix.calls.state !== "idle"
    readonly property bool front: camera.position === Camera.FrontFace
    /// The settings' word as the camera's mode; a front camera has no flash.
    readonly property int flashMode: {
        if (front) {
            return Camera.FlashOff
        }
        switch (settings.cameraFlash) {
        case "on": return Camera.FlashOn
        case "off": return Camera.FlashOff
        default: return Camera.FlashAuto
        }
    }
    readonly property bool live: status === PageStatus.Active && Qt.application.active
                                 && shot.length === 0 && !callRunning && failure.length === 0

    allowedOrientations: Orientation.Portrait

    function capture() {
        if (page.capturing || camera.cameraStatus !== Camera.ActiveStatus) {
            return
        }
        var path = matrix.cameraShots.newPath()
        if (path.length === 0) {
            page.failure = qsTr("The photo could not be saved.")
            return
        }
        camera.metaData.orientation = camera.position === Camera.FrontFace
                ? (720 + camera.orientation - page.pictureRotation) % 360
                : (720 + camera.orientation + page.pictureRotation) % 360
        page.capturing = true
        camera.imageCapture.captureToLocation(path)
    }

    /// Auto, on, off, and round again.
    function nextFlash() {
        settings.cameraFlash = settings.cameraFlash === "auto" ? "on"
                             : settings.cameraFlash === "on" ? "off" : "auto"
    }

    function focusAt(x, y) {
        var point = viewfinder.mapPointToSourceNormalized(Qt.point(x, y))
        if (point.x < 0 || point.x > 1 || point.y < 0 || point.y > 1) {
            return
        }
        camera.focus.focusPointMode = Camera.FocusPointCustom
        camera.focus.customFocusPoint = point
        camera.searchAndLock()
        focusMark.x = x - focusMark.width / 2
        focusMark.y = y - focusMark.height / 2
        focusMark.opacity = 1
        focusFade.restart()
    }

    function releaseFocus() {
        camera.unlock()
        camera.focus.focusPointMode = Camera.FocusPointAuto
    }

    function retake() {
        matrix.cameraShots.discard(page.shot)
        page.shot = ""
    }

    function use() {
        if (page.keepInGallery) {
            var stamp = Qt.formatDateTime(new Date(), "yyyyMMdd-hhmmss")
            matrix.saveToPictures(page.shot, "xmatic-" + stamp + ".jpg")
        }
        page.handedOver = true
        page.taken(page.shot)
        pageStack.pop()
    }

    // Leaving without "Use" takes the photo with it.
    Component.onDestruction: {
        if (!page.handedOver && page.shot.length > 0) {
            matrix.cameraShots.discard(page.shot)
        }
    }

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    Camera {
        id: camera

        captureMode: Camera.CaptureStillImage
        position: Camera.BackFace
        flash.mode: page.flashMode
        onPositionChanged: digitalZoom = 1
        // Released whenever nobody looks: another page, the app behind, a call.
        cameraState: page.live || page.capturing ? Camera.ActiveState : Camera.UnloadedState

        imageCapture {
            onImageSaved: {
                page.capturing = false
                page.releaseFocus()
                page.shot = path
            }
            onCaptureFailed: {
                page.capturing = false
                page.failure = qsTr("The photo could not be taken.")
                console.warn("xmatic: capture failed:", message)
            }
        }

        onError: {
            page.capturing = false
            page.failure = qsTr("The camera is not available.")
            console.warn("xmatic: camera error:", camera.errorString)
        }
    }

    OrientationSensor {
        active: page.live
        onReadingChanged: {
            switch (reading.orientation) {
            case OrientationReading.TopUp: page.pictureRotation = 0; break
            case OrientationReading.TopDown: page.pictureRotation = 180; break
            case OrientationReading.LeftUp: page.pictureRotation = 270; break
            case OrientationReading.RightUp: page.pictureRotation = 90; break
            default: break
            }
        }
    }

    VideoOutput {
        id: viewfinder

        anchors.fill: parent
        source: camera
        visible: page.shot.length === 0

        // Two fingers zoom, one tap focuses there.
        PinchArea {
            property real startZoom: 1

            anchors.fill: parent
            enabled: page.live
            onPinchStarted: startZoom = camera.digitalZoom
            onPinchUpdated: {
                camera.digitalZoom = Math.max(1, Math.min(camera.maximumDigitalZoom,
                                                          startZoom * pinch.scale))
                zoomLabel.opacity = 1
                zoomFade.restart()
            }

            MouseArea {
                anchors.fill: parent
                onClicked: page.focusAt(mouse.x, mouse.y)
            }
        }

        // Thirds, over the picture rather than the page.
        Item {
            x: viewfinder.contentRect.x
            y: viewfinder.contentRect.y
            width: viewfinder.contentRect.width
            height: viewfinder.contentRect.height
            visible: settings.cameraGrid

            Repeater {
                model: 2

                Rectangle {
                    x: Math.round(parent.width * (index + 1) / 3)
                    width: 1
                    height: parent.height
                    color: Qt.rgba(1, 1, 1, 0.5)
                }
            }

            Repeater {
                model: 2

                Rectangle {
                    y: Math.round(parent.height * (index + 1) / 3)
                    width: parent.width
                    height: 1
                    color: Qt.rgba(1, 1, 1, 0.5)
                }
            }
        }

        Rectangle {
            id: focusMark

            width: Theme.itemSizeMedium
            height: width
            radius: width / 2
            color: "transparent"
            border.color: camera.lockStatus === Camera.Locked ? Theme.highlightColor : "white"
            border.width: Math.max(2, Math.round(Theme.paddingSmall / 2))
            opacity: 0

            Behavior on opacity { FadeAnimation {} }
        }

        Timer {
            id: focusFade

            interval: 1500
            onTriggered: focusMark.opacity = 0
        }

        Label {
            id: zoomLabel

            anchors.centerIn: parent
            opacity: 0
            color: "white"
            font.pixelSize: Theme.fontSizeExtraLarge
            text: camera.digitalZoom.toFixed(1) + "×"

            Behavior on opacity { FadeAnimation {} }
        }

        Timer {
            id: zoomFade

            interval: 1000
            onTriggered: zoomLabel.opacity = 0
        }
    }

    // Flash and grid, out from under the cutout.
    Row {
        anchors {
            top: parent.top
            topMargin: Screen.topCutout.height + Theme.paddingMedium
            horizontalCenter: parent.horizontalCenter
        }
        spacing: Theme.paddingLarge
        visible: page.shot.length === 0 && page.failure.length === 0 && !page.callRunning

        IconButton {
            visible: !page.front
            icon.source: settings.cameraFlash === "on" ? "image://theme/icon-camera-flash-on"
                       : settings.cameraFlash === "off" ? "image://theme/icon-camera-flash-off"
                       : "image://theme/icon-camera-flash-automatic"
            onClicked: page.nextFlash()
        }

        IconButton {
            icon.source: settings.cameraGrid ? "image://theme/icon-camera-grid-thirds"
                                             : "image://theme/icon-camera-grid-none"
            onClicked: settings.cameraGrid = !settings.cameraGrid
        }
    }

    Image {
        anchors.fill: parent
        visible: page.shot.length > 0
        fillMode: Image.PreserveAspectFit
        autoTransform: true
        asynchronous: true
        // Bounded by the screen: a camera frame is many times its size.
        sourceSize.width: Screen.width
        sourceSize.height: Screen.height
        source: page.shot.length > 0 ? "file://" + page.shot : ""
    }

    Label {
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.horizontalPageMargin
        visible: page.failure.length > 0 || (page.callRunning && page.shot.length === 0)
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        color: Theme.highlightColor
        font.pixelSize: Theme.fontSizeLarge
        text: page.failure.length > 0 ? page.failure
                                      : qsTr("The camera is in use by the call.")
    }

    // Taking: switch and shutter.
    Item {
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge
        }
        height: shutter.height
        visible: page.shot.length === 0 && page.failure.length === 0 && !page.callRunning

        IconButton {
            id: shutter

            anchors.horizontalCenter: parent.horizontalCenter
            enabled: !page.capturing && camera.cameraStatus === Camera.ActiveStatus
            icon.source: "image://theme/icon-camera-shutter-release"
            onClicked: page.capture()
        }

        IconButton {
            anchors {
                right: parent.right
                rightMargin: Theme.horizontalPageMargin
                verticalCenter: parent.verticalCenter
            }
            enabled: !page.capturing
            icon.source: "image://theme/icon-camera-switch"
            onClicked: camera.position = camera.position === Camera.FrontFace
                                         ? Camera.BackFace : Camera.FrontFace
        }
    }

    // Looking at it: keep or not, again or use.
    Column {
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
            bottomMargin: Theme.paddingLarge
        }
        visible: page.shot.length > 0
        spacing: Theme.paddingMedium

        Rectangle {
            width: parent.width
            height: keep.height
            color: Qt.rgba(0, 0, 0, 0.6)

            TextSwitch {
                id: keep

                width: parent.width
                checked: page.keepInGallery
                text: qsTr("Keep in the gallery")
                description: qsTr("Otherwise the photo is deleted once it is sent.")
                onCheckedChanged: page.keepInGallery = checked
            }
        }

        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Theme.paddingLarge

            Button {
                text: qsTr("Retake")
                onClicked: page.retake()
            }

            Button {
                text: qsTr("Use")
                onClicked: page.use()
            }
        }
    }
}
