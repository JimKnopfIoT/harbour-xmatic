.pragma library

// A link held in a message's text: what is under the finger, and where it runs.

var step = 3

/// The link under a point of `item`, "" elsewhere; plain text carries none.
function linkAt(label, item, x, y) {
    if (!label.visible) {
        return ""
    }
    var p = label.mapFromItem(item, x, y)
    if (p.x < 0 || p.y < 0 || p.x > label.width || p.y > label.height) {
        return ""
    }
    return label.linkAt(p.x, p.y)
}

/// The held link's extent per line, in label coordinates. Text has no geometry for a link,
/// so the run is swept with linkAt: from the press, then across wrapped lines.
function sweep(label, item, link, x, y) {
    var lines = Math.max(1, label.lineCount)
    var lineHeight = label.contentHeight / lines
    var press = label.mapFromItem(item, x, y)
    var held = Math.min(lines - 1, Math.floor(press.y / lineHeight))
    var hit = function(px, line) {
        return label.linkAt(px, (line + 0.5) * lineHeight) === link
    }
    var marks = []
    var mark = function(line, left, right) {
        marks.push({ "x": left - step, "y": line * lineHeight,
                     "width": right - left + 2 * step, "height": lineHeight })
    }
    var left = press.x
    var right = press.x
    while (left - step >= 0 && hit(left - step, held)) {
        left -= step
    }
    while (right + step <= label.width && hit(right + step, held)) {
        right += step
    }
    mark(held, left, right)
    // Wrapped upwards: this run starts the line, the line above ends with it.
    var line = held
    var start = left
    var end
    while (start < step && line > 0) {
        end = label.width
        while (end >= 0 && label.linkAt(end, (line - 0.5) * lineHeight) === "") {
            end -= step
        }
        if (end < 0 || !hit(end, line - 1)) {
            break
        }
        start = end
        while (start - step >= 0 && hit(start - step, line - 1)) {
            start -= step
        }
        line--
        mark(line, start, end)
    }
    // Wrapped downwards: the next line starts with it.
    line = held
    while (line + 1 < lines && hit(0, line + 1)) {
        line++
        end = 0
        while (end + step <= label.width && hit(end + step, line)) {
            end += step
        }
        mark(line, 0, end)
        if (end + step <= label.width) {
            break
        }
    }
    return marks
}
