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

function linkify(body, clickableLinks) {
    var escaped = escapeText(body)
    // One pass over both kinds, web address first: a matrix.to permalink carries
    // a room address inside itself.
    var pattern = /(https?:\/\/[^\s<>"]+)|([#@!][A-Za-z0-9._=\-\/+]+:[A-Za-z0-9.\-]+(?::[0-9]+)?)/g
    return escaped.replace(pattern, function(match, url, address) {
        var trail = ""
        var target = url || address
        // Sentence punctuation glued to the end is not part of the link.
        var punctuation = target.match(/[.,;:!?]+$/)
        if (punctuation) {
            trail = punctuation[0]
            target = target.slice(0, target.length - trail.length)
        }
        if (url) {
            // A closing parenthesis only when none was opened.
            if (target.charAt(target.length - 1) === ")" && target.indexOf("(") < 0) {
                target = target.slice(0, target.length - 1)
                trail = ")" + trail
            }
            // A Matrix permalink is handled in the app, so it stays tappable even where
            // web links are off - it never reaches a browser.
            if (!clickableLinks && !MatrixLinks.parse(target)) {
                return target + trail
            }
            // The href as written, not as shown: `target` left the escaper with
            // `&amp;`, and Qt decodes no entities in an attribute.
            var href = safeHref(target.replace(/&amp;/g, "&"))
            if (href.length === 0) {
                return target + trail
            }
            return "<a href=\"" + href + "\">" + target + "</a>" + trail
        }
        return "<a href=\"xmatic:" + target + "\">" + target + "</a>" + trail
    })
}

/// Only a body that visibly carries a link pays the rich-text path, behind a setting.
function hasLink(body, clickableLinks) {
    return (clickableLinks && /https?:\/\//.test(body || ""))
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
    var found = /https?:\/\/[^\s<>"]+/.exec(body || "")
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
    return safeHref(address)
}
