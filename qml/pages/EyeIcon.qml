import QtQuick 2.0
import Sailfish.Silica 1.0

// An open eye - how many read a message. Drawn like FaceIcon: follows every
// ambience colour and needs no zoom levels kept in step with the theme.
Item {
    id: eye

    property real size: Theme.iconSizeExtraSmall
    property color color: Theme.secondaryColor

    width: size
    height: size
    opacity: Theme.opacityLow

    onColorChanged: canvas.requestPaint()
    onSizeChanged: canvas.requestPaint()

    Canvas {
        id: canvas

        anchors.fill: parent

        // Kept as an image, not a framebuffer, and repainted on return: a Canvas hands
        // its framebuffer back when the window leaves and comes back empty.
        renderTarget: Canvas.Image

        // A canvas draws nothing while invisible, and delegates are recycled - one
        // coming back showed the number without its picture.
        onVisibleChanged: {
            if (visible) {
                requestPaint()
            }
        }

        Connections {
            target: Qt.application
            onActiveChanged: {
                if (Qt.application.active) {
                    canvas.requestPaint()
                }
            }
        }

        onPaint: {
            var ctx = getContext("2d")
            ctx.reset()

            var cx = width / 2
            var cy = height / 2
            var half = eye.size * 0.40
            // The control point, not the height the line reaches: a quadratic curve comes
            // half way up to it, and the literal reading drew an eye twice as wide as tall.
            var lid = eye.size * 0.48

            ctx.lineWidth = Math.max(1, Math.round(eye.size / 24))
            ctx.strokeStyle = eye.color

            // Two arcs meeting at the corners - the almond.
            ctx.beginPath()
            ctx.moveTo(cx - half, cy)
            ctx.quadraticCurveTo(cx, cy - lid, cx + half, cy)
            ctx.quadraticCurveTo(cx, cy + lid, cx - half, cy)
            ctx.stroke()

            // The pupil outlined, not filled: a dot at this size turns the eye
            // into a blot.
            ctx.beginPath()
            ctx.arc(cx, cy, eye.size * 0.14, 0, 2 * Math.PI)
            ctx.stroke()
        }
    }
}
