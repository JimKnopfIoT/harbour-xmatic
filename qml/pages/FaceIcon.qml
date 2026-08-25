import QtQuick 2.0
import Sailfish.Silica 1.0

// A round, smiling face. Silica ships no such icon, so it is drawn instead of
// shipped: drawing follows every ambience colour on its own and needs no set
// of zoom levels kept in step with the theme's.
//
// Stroke width and opacity are matched to the theme's own line icons next to
// it - a heavier outline next to the send arrow reads as a different kind of
// control, not as the same row of buttons.
Item {
    id: face

    property real size: Theme.iconSizeMedium
    property color color: Theme.primaryColor

    width: size
    height: size
    // One step paler than the theme's line icons: the face is an offer, not a
    // control that wants attention.
    opacity: Theme.opacityLow

    onColorChanged: canvas.requestPaint()
    onSizeChanged: canvas.requestPaint()

    Canvas {
        id: canvas

        anchors.fill: parent

        onPaint: {
            var ctx = getContext("2d")
            ctx.reset()

            var cx = width / 2
            var cy = height / 2
            var r = face.size * 0.42
            var stroke = Math.max(1, Math.round(face.size / 24))

            ctx.lineWidth = stroke
            ctx.strokeStyle = face.color
            ctx.fillStyle = face.color

            ctx.beginPath()
            ctx.arc(cx, cy, r, 0, 2 * Math.PI)
            ctx.stroke()

            var eye = Math.max(1, face.size / 24)
            ctx.beginPath()
            ctx.arc(cx - r * 0.38, cy - r * 0.30, eye, 0, 2 * Math.PI)
            ctx.fill()
            ctx.beginPath()
            ctx.arc(cx + r * 0.38, cy - r * 0.30, eye, 0, 2 * Math.PI)
            ctx.fill()

            // The mouth is an arc of the lower half, open upwards.
            ctx.beginPath()
            ctx.arc(cx, cy + r * 0.02, r * 0.55, 0.2 * Math.PI, 0.8 * Math.PI)
            ctx.stroke()
        }
    }
}
