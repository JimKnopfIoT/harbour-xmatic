.pragma library

/// Percent-decoding that cannot throw: `decodeURIComponent("%zz")` raises, and
/// a raise inside a binding leaves the row half-built.
function safeDecode(text) {
    try {
        return decodeURIComponent(text)
    } catch (error) {
        return text
    }
}

// What a link in a message points at, and how the app deals with it.
//
// Matrix carries its own addresses: `#room:server`, `@user:server`, and the
// permalinks of matrix.to and the `matrix:` URI scheme. Handing those to the
// browser is what happened before, and the browser can do nothing with them -
// the room stays out of reach. They are recognised here and handled inside the
// app instead.
//
// Nothing here acts by itself. A tapped link may open a room this account is
// already in; anything else ends in a dialog the user confirms. A message must
// not be able to put its reader anywhere.

// A Matrix address inside plain text: sigil, a name, a colon, a server. The
// server may carry a port. Deliberately narrow - an e-mail address has no
// leading sigil and no colon, so it cannot match.
var ADDRESS = /([#@!][A-Za-z0-9._=\-\/+]+:[A-Za-z0-9.\-]+(?::[0-9]+)?)/

// Anything this app writes as a link for its own use.
var INTERNAL_PREFIX = "xmatic:"

// Reads a link and says what it means: { kind: "room"|"user", id: "..." },
// or null for an ordinary web link.
function parse(link) {
    if (!link) {
        return null
    }
    var target = ""
    if (link.indexOf(INTERNAL_PREFIX) === 0) {
        target = link.substring(INTERNAL_PREFIX.length)
    } else if (link.indexOf("matrix:") === 0) {
        // matrix:r/room:server, matrix:roomid/abc:server, matrix:u/user:server
        var rest = link.substring("matrix:".length).split("?")[0]
        var parts = rest.split("/")
        if (parts.length < 2) {
            return null
        }
        var sigils = { "r": "#", "roomid": "!", "u": "@" }
        if (!sigils[parts[0]]) {
            return null
        }
        target = sigils[parts[0]] + safeDecode(parts[1])
    } else {
        // The host has to *be* matrix.to, not merely contain it:
    // https://tracker.example/px?u=matrix.to/#/x was classed as internal and
    // stayed tappable with web links switched off.
    var marker = /^https?:\/\/(www\.)?matrix\.to\/#\//i.test(link)
                 ? link.indexOf("matrix.to/#/") : -1
        if (marker < 0) {
            return null
        }
        target = safeDecode(link.substring(marker + "matrix.to/#/".length))
        // A permalink can name an event inside the room, and carries via
        // servers as a query. Neither is used here: the room is the target.
        target = target.split("?")[0].split("/")[0]
    }

    if (target.length < 3) {
        return null
    }
    if (target.charAt(0) === "#" || target.charAt(0) === "!") {
        return { "kind": "room", "id": target }
    }
    if (target.charAt(0) === "@") {
        return { "kind": "user", "id": target }
    }
    return null
}

// Whether a plain body carries something worth making tappable.
function hasAddress(body) {
    return ADDRESS.test(body || "")
}
