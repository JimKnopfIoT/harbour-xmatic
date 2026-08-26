import QtQuick 2.0
import Sailfish.Silica 1.0

// A padlock, closed or struck through. Drawn rather than taken from the theme
// for the same reason FaceIcon and EyeIcon are: it follows every ambience
// colour, needs no set of zoom levels, and can be drawn as thinly as this one
// has to be.
//
// It is a watermark, not a control: it sits behind the message field at a
// tenth of the theme's opacity, where the room's own state can be seen without
// being read. Whether a room is encrypted is stated in words in the header;
// this is the reminder while typing, and it must never compete with the text
// over it.
Item {
    id: lock

    property real size: Theme.iconSizeLarge
    property color color: Theme.primaryColor
    /// Closed for an encrypted room, struck through for one without.
    property bool locked: true

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

            // One hairline, whatever the size: a watermark that thickens with
            // the icon starts to read as a drawing.
            ctx.lineWidth = Math.max(1, Math.round(s / 40))
            ctx.strokeStyle = lock.color

            // The body, and the shackle standing on its top edge.
            var bodyWidth = s * 0.52
            var bodyHeight = s * 0.38
            var bodyTop = cy - s * 0.06
            ctx.beginPath()
            ctx.rect(cx - bodyWidth / 2, bodyTop, bodyWidth, bodyHeight)
            ctx.stroke()

            ctx.beginPath()
            ctx.arc(cx, bodyTop, s * 0.18, Math.PI, 2 * Math.PI)
            ctx.stroke()

            if (!lock.locked) {
                // Struck through rather than hanging open: an open shackle is
                // a difference of a few pixels at this weight, a line across
                // the whole icon is not.
                //
                // Top left down to bottom right, the way every prohibition
                // sign in the world is drawn - the other diagonal reads as a
                // slip of the hand. It starts level with the top of the
                // shackle and ends at the bottom of the body, so nothing of it
                // sticks out beyond the lock itself.
                ctx.beginPath()
                ctx.moveTo(cx - bodyWidth * 0.72, bodyTop - s * 0.18)
                ctx.lineTo(cx + bodyWidth * 0.72, bodyTop + bodyHeight)
                ctx.stroke()
            }
        }
    }
}
