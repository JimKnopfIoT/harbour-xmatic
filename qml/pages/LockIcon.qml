import QtQuick 2.0
import Sailfish.Silica 1.0

// A padlock, closed or open, at the top of the room: the shape carries it, not
// a colour - red and green over a room name shout down the name.
Item {
    id: lock

    property real size: Theme.iconSizeMedium
    /// Closed for an encrypted room, open for one without.
    property bool locked: true
    /// The theme's ink, like every other mark in this app.
    property color color: Theme.primaryColor

    width: size
    height: size

    onColorChanged: canvas.requestPaint()
    onSizeChanged: canvas.requestPaint()
    onLockedChanged: canvas.requestPaint()

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

            var s = lock.size
            var cx = width / 2
            var cy = height / 2

            // The same hairline the face in the composer uses: one stroke for every mark
            // this app draws itself, so they read as one hand.
            ctx.lineWidth = Math.max(1, Math.round(s / 24))
            ctx.strokeStyle = lock.color
            ctx.lineJoin = "round"
            ctx.lineCap = "round"

            // The open lock stands wider than the closed one, so its body
            // steps aside to keep the whole figure inside the icon's box.
            var bodyWidth = s * 0.52
            var bodyHeight = s * 0.34
            var bodyTop = cy + s * 0.04
            var bodyCx = lock.locked ? cx : cx - s * 0.10
            ctx.beginPath()
            ctx.rect(bodyCx - bodyWidth / 2, bodyTop, bodyWidth, bodyHeight)
            ctx.stroke()

            // The shackle as an arch on two straight legs: the legs are what make it
            // stand clear of the body at this size.
            var r = lock.locked ? s * 0.2 : s * 0.18
            var neck = r * 0.5

            if (lock.locked) {
                ctx.beginPath()
                ctx.moveTo(bodyCx - r, bodyTop)
                ctx.lineTo(bodyCx - r, bodyTop - neck)
                ctx.arc(bodyCx, bodyTop - neck, r, Math.PI, 2 * Math.PI)
                ctx.lineTo(bodyCx + r, bodyTop)
                ctx.stroke()
                return
            }

            // Open: the same arch, lifted out of the body and set beside it - where a
            // shackle goes when it holds nothing.
            ctx.save()
            ctx.translate(bodyCx + r * 0.85, bodyTop - neck * 0.9)
            ctx.beginPath()
            ctx.moveTo(0, 0)
            ctx.lineTo(0, -neck)
            ctx.arc(r, -neck, r, Math.PI, 2 * Math.PI)
            ctx.lineTo(2 * r, 0)
            ctx.stroke()
            ctx.restore()

            // And struck through corner to corner: the shape alone is a small difference
            // on a small screen, a line across it is not.
            ctx.beginPath()
            ctx.moveTo(bodyCx - bodyWidth / 2, bodyTop)
            ctx.lineTo(bodyCx + bodyWidth / 2, bodyTop + bodyHeight)
            ctx.stroke()
        }
    }
}
