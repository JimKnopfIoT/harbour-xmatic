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

// Matrix addresses and permalinks are recognised here and handled in the app -
// a browser can do nothing with them. Nothing here acts by itself.

// A Matrix address in plain text: sigil, name, colon, server, optional port.
// Narrow on purpose - an e-mail address cannot match.
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
        // The host has to *be* matrix.to, not contain it: a tracker URL with it in a
        // query was classed as internal and stayed tappable.
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

/// Where a tapped link leads, decided in one place. The allowlist for web
/// addresses lives here and nowhere else: two copies of it drift apart, and
/// this one decides whether a stranger's string reaches a browser.
/// Kinds: "room", "user", "web", "none". Acting on it belongs to the page.
function decide(link) {
    var target = parse(link)
    if (target) {
        return target
    }
    if (/^https?:\/\//i.test(link)) {
        return { "kind": "web", "id": link }
    }
    return { "kind": "none", "id": "" }
}
