.pragma library

.import "Formatting.js" as Formatting
.import "MatrixLinks.js" as MatrixLinks
// A message's text as the room and the thread draw it: one rule, one place.

/// Everything a sender wrote loses its meaning as markup here, and only
/// here. Whatever is appended afterwards was written by this app.
function escapeText(body) {
    return body.replace(/&/g, "&amp;")
               .replace(/</g, "&lt;")
               .replace(/>/g, "&gt;")
}

/// A URL inside an href, where Qt decodes no entities: the ampersand has to
/// stay itself. Anything that could end the attribute is refused.
function safeHref(url) {
    if (/["'<>`\\\s]/.test(url)) {
        return ""
    }
    // The same rule the core applies to a formatted link: an address is
    // ASCII. Anything else is invisible, a homograph, or both — and this
    // one is drawn from the plain body, which keeps its zero-width
    // characters because an emoji sequence needs them.
    if (/[^\x20-\x7e]/.test(url)) {
        return ""
    }
    // Everything before an "@" in the authority is a user name; the host
    // that decides sits behind it, past where the dialog wraps.
    var authority = url.replace(/^https?:\/\//i, "").split(/[\/?#]/)[0]
    return authority.indexOf("@") >= 0 ? "" : url
}

/// The scheme as a browser expects it: the keyboard capitalises the first letter.
function lowerScheme(url) {
    return url.replace(/^https?:/i, function(scheme) { return scheme.toLowerCase() })
}

// Endings an address without scheme may carry. Those that double as file
// extensions stay out (README.md, main.rs, setup.py, run.sh, lib.so, logo.ai, x.gd).
var BARE_ENDINGS = (function() {
    var list = ("com net org biz edu gov int eu io dev xyz online site shop store tech cloud blog"
        + " news social chat space art page live club top ad ae af ag al ao aq ar at au aw ax az"
        + " ba bb bd be bf bg bh bi bj bm bn bo br bs bt bw by ca cd cf cg ch ci ck cm cn co cu"
        + " cv cw cx cy cz de dj dk dm dz ec ee eg er es et fi fj fk fm fo fr ga ge gf gg gh gi"
        + " gl gm gn gp gq gr gt gu gw gy hk hm hn hr ht hu id ie il im iq ir is it je jm jo jp"
        + " ke kg kh ki km kn kp kr kw ky kz lb lc li lk lr ls lt lu lv ma mc me mg mh mn mp mq"
        + " mr mt mu mv mw mx my mz na nc ne nf ng ni nl no np nr nu nz om pa pe pf pg ph pk pn"
        + " pr pw qa ro ru rw sa sb sd se sg si sk sl sm sn sr ss su sx sy sz tc td tg th tj tk"
        + " tl tm tn to tr tt tv tw tz ua ug uk us uy uz va vc ve vg vn vu wf ws ye yt za zm zw").split(" ")
    var endings = {}
    for (var i = 0; i < list.length; ++i) {
        endings[list[i]] = true
    }
    return endings
})()

// Anchored, and only ever run on one word: unanchored over a long run of letters
// the search restarts at every position, which a stranger's message can make slow.
var BARE_HOST = /^(?:[A-Za-z0-9](?:[A-Za-z0-9\-]{0,61}[A-Za-z0-9])?\.){1,8}([a-z]{2,24})(?::[0-9]{1,5})?(?:[\/?#][^\s<>"]*)?/
var WORD = /[^\s<>"]+/g
var LONGEST_WORD = 2048

/// The address a word starts with, after an opening bracket or quote; null for a
/// mail address, a path, a file name or anything glued to more text.
function bareIn(word) {
    if (word.length > LONGEST_WORD || word.indexOf(".") < 0) {
        return null
    }
    var lead = word.match(/^(?:&lt;|[(\[{'*«“„])*/)[0]
    var rest = word.slice(lead.length)
    var found = BARE_HOST.exec(rest)
    if (!found || !BARE_ENDINGS.hasOwnProperty(found[1])
            || /^[A-Za-z0-9_\-=+%~@]/.test(rest.slice(found[0].length))) {
        return null
    }
    return { lead: lead, address: found[0], tail: rest.slice(found[0].length) }
}

function hasBareAddress(body) {
    WORD.lastIndex = 0
    var word
    while ((word = WORD.exec(body)) !== null) {
        if (bareIn(word[0])) {
            return true
        }
    }
    return false
}

/// `target` with the sentence's punctuation and an unopened bracket moved out.
/// It is escaped text: `&lt;`/`&gt;` end it, and an entity is never cut in half.
function splitTrail(target, url) {
    var trail = ""
    var bracket = target.search(/&[lg]t;/)
    if (bracket >= 0) {
        trail = target.slice(bracket)
        target = target.slice(0, bracket)
    }
    for (;;) {
        if (/&amp;$/.test(target)) {
            trail = "&amp;" + trail
            target = target.slice(0, target.length - 5)
        } else if (target.length > 0 && ".,;:!?".indexOf(target.charAt(target.length - 1)) >= 0) {
            trail = target.charAt(target.length - 1) + trail
            target = target.slice(0, target.length - 1)
        } else {
            break
        }
    }
    if (url && target.charAt(target.length - 1) === ")" && target.indexOf("(") < 0) {
        target = target.slice(0, target.length - 1)
        trail = ")" + trail
    }
    return { target: target, trail: trail }
}

/// A web anchor, or the text itself where the address is refused. `full` is the
/// href as escaped; Qt decodes no entities in an attribute.
function webAnchor(shown, full) {
    var href = safeHref(full.replace(/&amp;/g, "&"))
    return href.length === 0 ? shown : "<a href=\"" + href + "\">" + shown + "</a>"
}

/// Bare addresses in the text between the anchors already made; a word that runs
/// into an anchor belongs to it and is left alone.
function linkBare(html) {
    var parts = html.split(/(<a [^>]*>[\s\S]*?<\/a>)/)
    for (var i = 0; i < parts.length; i += 2) {
        var text = parts[i]
        var afterAnchor = i > 0
        var beforeAnchor = i < parts.length - 1
        parts[i] = text.replace(WORD, function(word, offset) {
            if ((afterAnchor && offset === 0)
                    || (beforeAnchor && offset + word.length === text.length)) {
                return word
            }
            var found = bareIn(word)
            if (!found) {
                return word
            }
            var cut = splitTrail(found.address, true)
            return found.lead + webAnchor(cut.target, "https://" + cut.target)
                    + cut.trail + found.tail
        })
    }
    return parts.join("")
}

function linkify(body, clickableLinks) {
    var escaped = escapeText(body)
    // One pass over both kinds, web address first: a matrix.to permalink carries
    // a room address inside itself.
    var pattern = /(https?:\/\/[^\s<>"]+)|([#@!][A-Za-z0-9._=\-\/+]+:[A-Za-z0-9.\-]+(?::[0-9]+)?)/gi
    var linked = escaped.replace(pattern, function(match, url, address) {
        var cut = splitTrail(url || address, !!url)
        if (url) {
            var full = lowerScheme(cut.target)
            // A Matrix permalink is handled in the app, so it stays tappable even where
            // web links are off - it never reaches a browser.
            if (!clickableLinks && !MatrixLinks.parse(full)) {
                return cut.target + cut.trail
            }
            return webAnchor(cut.target, full) + cut.trail
        }
        return "<a href=\"xmatic:" + cut.target + "\">" + cut.target + "</a>" + cut.trail
    })
    return clickableLinks ? linkBare(linked) : linked
}

/// Only a body that visibly carries a link pays the rich-text path, behind a setting.
function hasLink(body, clickableLinks) {
    return (clickableLinks && (/https?:\/\//i.test(body || "") || hasBareAddress(body || "")))
            || MatrixLinks.hasAddress(body)
}

/// The body as markup, or empty where plain text will do. `formatted` is the core's
/// markup, which escaped the sender's characters itself. Never for a file row.
function rich(body, formatted, settings, codeColour, emojiPixels, emojiSource) {
    var base = ""
    var linked = false
    if ((formatted || "").length > 0) {
        base = Formatting.renderFormatted(formatted, settings.clickableLinks, codeColour)
        linked = true
    } else if (hasLink(body, settings.clickableLinks)) {
        base = linkify(body || "", settings.clickableLinks)
        linked = true
    } else if (settings.emojiImages) {
        base = escapeText(body || "")
    } else {
        return ""
    }
    if (!settings.emojiImages) {
        return base
    }
    var pictured = Formatting.withEmojiPictures(base, emojiPixels, emojiSource)
    // A plain body with no picture in it stays plain text -
    // the markup renderer costs more and buys nothing.
    if (!linked && pictured === base) {
        return ""
    }
    return pictured
}

/// The first web address in the text, where the setting allows a preview. Empty means
/// no card - and no question to the server. Pass `encrypted` true while unknown.
function previewUrl(body, linkPreviews, encrypted) {
    if (linkPreviews === "never" || (linkPreviews === "unencrypted" && encrypted)) {
        return ""
    }
    // The same alphabet linkify uses, so the card describes the address a tap
    // on the text would open.
    var found = /https?:\/\/[^\s<>"]+/i.exec(body || "")
    if (!found) {
        return ""
    }
    var address = found[0].replace(/[.,;:!?]+$/, "")
    // A closing bracket without its opener belongs to the sentence.
    if (/\)$/.test(address) && address.indexOf("(") < 0) {
        address = address.slice(0, -1)
    }
    // Through the same gate as the link in the text: an address the text refused
    // must not come back as a card, nor be fetched by the homeserver.
    return safeHref(lowerScheme(address))
}
