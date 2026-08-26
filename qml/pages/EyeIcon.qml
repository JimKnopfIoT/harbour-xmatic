import QtQuick 2.0
import Sailfish.Silica 1.0

// An open eye: how many people have read a message. Silica ships no such icon,
// so it is drawn for the same reasons FaceIcon is - drawing follows every
// ambience colour and needs no set of zoom levels kept in step with the theme.
//
// Stroke width and opacity are FaceIcon's, so the two read as one hand: a thin
// outline at the theme's low opacity, a mark beside the text rather than a
// control that wants attention.
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

        // Kept as an image, not as a framebuffer, and repainted when the app
        // comes back to the front. A Canvas hands its framebuffer back when the
        // window leaves the screen, and what returns is empty until something
        // asks for a repaint - the mark was there on entering a room, gone
        // after a trip through the tile view, and back again after leaving the
        // room and returning, which is when the page rebuilt it. Nothing else
        // in this row is drawn, which is why nothing else showed it.
        renderTarget: Canvas.Image

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
            // The control point, not the height the line reaches: a quadratic
            // curve comes half way up to it. Written as the height it looked
            // right and drew an eye twice as wide as it was tall.
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
