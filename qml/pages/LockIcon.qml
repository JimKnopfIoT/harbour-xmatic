import QtQuick 2.0
import Sailfish.Silica 1.0

// A padlock, closed or standing open. Drawn rather than taken from the theme
// for the same reason FaceIcon and EyeIcon are: it needs no set of zoom
// levels and it can be drawn in the two shapes this app needs.
//
// It says one thing, at the top of the room next to its name: whether what is
// written here is encrypted. Closed for an encrypted room, open for one
// without - the shackle swung out and the body struck through. The shape
// carries it, not a colour: red and green over a room name shout louder than
// everything else in the strip, and the strip is there for the name. That is
// why the room header no longer spells it out in words either.
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

        // Kept as an image, not as a framebuffer, and repainted when the app
        // comes back to the front. A Canvas hands its framebuffer back when the
        // window leaves the screen, and what returns is empty until something
        // asks for a repaint - the mark was there on entering a room, gone
        // after a trip through the tile view, and back again after leaving the
        // room and returning, which is when the page rebuilt it. Nothing else
        // in this row is drawn, which is why nothing else showed it.
        renderTarget: Canvas.Image

        // A canvas draws nothing while it is invisible, and the delegates of
        // a conversation are recycled: one that comes back showing a mark it
        // did not show before had an empty canvas over it - the number stood
        // there without its picture. Asking for the paint when it becomes
        // visible costs nothing where it was visible all along.
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

            // The same hairline the face in the composer is drawn with
            // (FaceIcon), by the same rule: one stroke for every mark this app
            // draws itself, so they read as one hand.
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

            // The shackle, an arch on two straight legs rather than a bare
            // half circle: the legs are what make it stand clear of the body
            // at this size.
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

            // Open: the same arch, lifted out of the body and set down beside
            // it - out and up, where a shackle goes when it is not holding
            // anything.
            ctx.save()
            ctx.translate(bodyCx + r * 0.85, bodyTop - neck * 0.9)
            ctx.beginPath()
            ctx.moveTo(0, 0)
            ctx.lineTo(0, -neck)
            ctx.arc(r, -neck, r, Math.PI, 2 * Math.PI)
            ctx.lineTo(2 * r, 0)
            ctx.stroke()
            ctx.restore()

            // And struck through, corner to corner across the body: the shape
            // alone would be a small difference on a small screen, a line
            // across it is not.
            ctx.beginPath()
            ctx.moveTo(bodyCx - bodyWidth / 2, bodyTop)
            ctx.lineTo(bodyCx + bodyWidth / 2, bodyTop + bodyHeight)
            ctx.stroke()
        }
    }
}
