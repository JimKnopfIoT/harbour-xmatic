//! Rewrites a message's `formatted_body` into markup `Text.StyledText` can draw.
//!
//! Not a sanitiser: ruma parses and filters to the Matrix subset, then the tree
//! is walked and re-emitted, so every `<` in the output is written here and
//! every sender character goes through the escaper. An unknown tag contributes
//! its text only.
//!
//! Qt's subset is much smaller than the spec's, hence the rewriting: no `<pre>`,
//! `<code>` or `<blockquote>`, and `<ul>` indents by a fixed pixel margin. Not
//! carried over at all: colours and sizes (a sender who picks the colour picks
//! the background's), `<img>` (a URL the sender chose, fetched on display), and
//! anchors to anything but http(s). `Text.RichText` is unusable for the same
//! image reason.
//!
//! Spoilers are marked, not hidden — StyledText cannot hide a run of text.

use ruma_html::{Html, HtmlSanitizerMode, SanitizerConfig};

/// Beyond this the plain body is used: the whole row ends up in one Qt label,
/// and a megabyte of markup in a list delegate is a frozen scroll.
const MAX_INPUT: usize = 64 * 1024;

/// Same, on the output: a small input can expand through nesting.
const MAX_OUTPUT: usize = 96 * 1024;

/// `None` when the result adds nothing over the plain body — the usual case,
/// since many clients attach a `formatted_body` to every message.
pub fn to_styled_text(html: &str) -> Option<String> {
    if html.is_empty() || html.len() > MAX_INPUT {
        return None;
    }

    let parsed = Html::parse(html);
    // The reply fallback goes: the quote is drawn from the event's relation, so
    // it would appear twice.
    //
    // `remove_elements` because a merely *disallowed* element is replaced by its
    // children — a `<script>` vanishes while its source stays behind as text.
    let config = SanitizerConfig::with_mode(HtmlSanitizerMode::Compat)
        .remove_reply_fallback()
        .remove_elements([
            "script", "style", "noscript", "template", "iframe", "object", "embed", "applet",
            "head", "title", "base", "link", "meta",
        ]);
    parsed.sanitize_with(&config);

    let mut writer = Writer::default();
    writer.children(parsed.children(), &Context::default());

    let markup = writer.markup;
    let out = writer.finish();
    if out.is_empty() || !markup {
        return None;
    }
    Some(out)
}

/// What the position in the tree changes about rendering.
#[derive(Clone, Copy, Default)]
struct Context {
    /// Inside `<pre>`: line breaks are kept and leading indentation survives.
    preformatted: bool,
    /// Inside `<a>`: no nested anchor.
    in_anchor: bool,
    /// Inside a monospace run. `<pre><code>` would otherwise open the font twice.
    monospace: bool,
    /// Guards this module's own recursion; ruma caps the tree at 100 levels.
    depth: u32,
}

const MAX_DEPTH: u32 = 64;

#[derive(Default)]
struct Writer {
    out: String,
    /// Whether anything was written that the plain body could not carry.
    markup: bool,
    /// Set once a cap was hit; everything after is dropped.
    truncated: bool,
}

impl Writer {
    fn finish(mut self) -> String {
        // A trailing break renders as an empty line under the message.
        while self.out.ends_with("<br>") {
            let len = self.out.len() - 4;
            self.out.truncate(len);
        }
        self.out
    }

    fn push_raw(&mut self, markup: &str) {
        if self.truncated {
            return;
        }
        if self.out.len() + markup.len() > MAX_OUTPUT {
            self.truncated = true;
            return;
        }
        self.out.push_str(markup);
    }

    /// Writes our own markup and marks the row as needing the rich-text path.
    fn push_tag(&mut self, tag: &str) {
        self.markup = true;
        self.push_raw(tag);
    }

    /// Writes sender-provided text, escaped so it can never become markup.
    ///
    /// Direction controls are dropped on the way (`text::strip_bidi`): they are
    /// invisible and reorder what follows, so a formatted message could show
    /// one thing and say another.
    fn push_text(&mut self, text: &str, context: &Context) {
        for character in text.chars() {
            if crate::text::is_bidi_control(character) {
                continue;
            }
            match character {
                '&' => self.push_raw("&amp;"),
                '<' => self.push_raw("&lt;"),
                '>' => self.push_raw("&gt;"),
                '\n' if context.preformatted => self.push_tag("<br>"),
                // Only leading spaces: `&nbsp;` everywhere would stop the
                // label wrapping, and a long code line would be cut off.
                ' ' if context.preformatted && self.at_indent() => self.push_raw("&nbsp;"),
                _ => {
                    let mut buffer = [0u8; 4];
                    let encoded: &str = character.encode_utf8(&mut buffer);
                    self.push_raw(encoded);
                }
            }
        }
    }

    fn at_line_start(&self) -> bool {
        self.out.is_empty() || self.out.ends_with("<br>")
    }

    /// Still in the leading whitespace — `&nbsp;` does not start the line.
    fn at_indent(&self) -> bool {
        self.at_line_start() || self.out.ends_with("&nbsp;")
    }

    /// Separates blocks, never doubling up.
    fn block_break(&mut self) {
        if self.out.is_empty() || self.at_line_start() {
            return;
        }
        self.push_tag("<br>");
    }

    fn children(&mut self, nodes: ruma_html::Children, context: &Context) {
        for node in nodes {
            self.node(&node, context);
        }
    }
}

/// Which of Qt's tags an element maps to, where the mapping is a plain wrap.
fn inline_wrapper(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "b" | "strong" => Some(("<b>", "</b>")),
        "i" | "em" | "cite" | "dfn" | "var" => Some(("<i>", "</i>")),
        "u" | "ins" => Some(("<u>", "</u>")),
        "s" | "del" | "strike" => Some(("<s>", "</s>")),
        "sub" => Some(("<sub>", "</sub>")),
        "sup" => Some(("<sup>", "</sup>")),
        // No `<code>` in StyledText. Generic family name: fontconfig resolves
        // it, a concrete font would be a guess about the device.
        "code" | "kbd" | "samp" | "tt" => Some(("<font family=\"monospace\">", "</font>")),
        _ => None,
    }
}

impl Writer {
    fn node(&mut self, node: &ruma_html::NodeRef, context: &Context) {
        if self.truncated || context.depth > MAX_DEPTH {
            return;
        }

        match node.data() {
            ruma_html::NodeData::Text(text) => {
                self.push_text(&text.borrow(), context);
            }
            ruma_html::NodeData::Element(element) => {
                self.element(node, &element.name.local, element, context);
            }
            // Document and comments: only their children matter.
            _ => {
                let inner = Context { depth: context.depth + 1, ..*context };
                self.children(node.children(), &inner);
            }
        }
    }

    fn element(
        &mut self,
        node: &ruma_html::NodeRef,
        name: &str,
        element: &ruma_html::ElementData,
        context: &Context,
    ) {
        let inner = Context { depth: context.depth + 1, ..*context };

        if let Some((open, close)) = inline_wrapper(name) {
            if context.monospace && open.starts_with("<font") {
                self.children(node.children(), &inner);
                return;
            }
            let inner = Context { monospace: open.starts_with("<font"), ..inner };
            self.push_tag(open);
            self.children(node.children(), &inner);
            self.push_tag(close);
            return;
        }

        match name {
            "br" => self.push_tag("<br>"),

            // No Qt equivalent; their contribution is the line break.
            "p" | "div" | "section" | "article" | "aside" | "caption" | "summary" | "details" => {
                self.block_break();
                self.children(node.children(), &inner);
                self.block_break();
            }

            // Not Qt's `<h1>`: it scales the font by a fixed factor, which in
            // a chat bubble is several times the conversation's size.
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.block_break();
                self.push_tag("<b>");
                self.children(node.children(), &inner);
                self.push_tag("</b>");
                self.block_break();
            }

            "hr" => {
                self.block_break();
                self.push_tag("————");
                self.block_break();
            }

            // Not Qt's `<ul>`/`<ol>`: it indents by a fixed pixel margin a
            // bubble does not have, and nesting loses its level.
            "ul" | "ol" => {
                self.block_break();
                self.list(node, name == "ol", element, &inner);
                self.block_break();
            }

            // Only a stray `<li>` outside a list; inside one, `list` handles it.
            "li" => {
                self.block_break();
                self.children(node.children(), &inner);
                self.block_break();
            }

            // No blockquote in StyledText; the bar is a character and has to
            // be repeated per line, so render first, then prefix.
            "blockquote" => {
                self.block_break();
                let mut quoted = Writer::default();
                quoted.children(node.children(), &inner);
                let markup = quoted.markup;
                let text = quoted.finish();
                if !text.is_empty() {
                    self.markup = self.markup || markup;
                    self.push_tag("▏ ");
                    self.push_raw(&text.replace("<br>", "<br>▏ "));
                }
                self.block_break();
            }

            "pre" => {
                self.block_break();
                let preformatted = Context { preformatted: true, monospace: true, ..inner };
                self.push_tag("<font family=\"monospace\">");
                self.children(node.children(), &preformatted);
                self.push_tag("</font>");
                self.block_break();
            }

            "a" => self.anchor(node, element, &inner),

            // A table cannot be one in a phone-width bubble; keep cells apart.
            "tr" => {
                self.block_break();
                self.children(node.children(), &inner);
                self.block_break();
            }
            "td" | "th" => {
                if !self.at_line_start() {
                    self.push_tag("  │  ");
                }
                self.children(node.children(), &inner);
            }

            // Never fetched — the sender picks the URL. Custom emoji included.
            "img" => {
                let alt = attribute(element, "alt")
                    .or_else(|| attribute(element, "title"))
                    .unwrap_or_default();
                if !alt.is_empty() {
                    self.push_text(&alt, context);
                }
            }

            // Carries the spoiler in the spec; its colours are ignored.
            "span" => {
                if attribute(element, "data-mx-spoiler").is_some() {
                    // Marked, not hidden.
                    self.push_tag("▨ ");
                }
                self.children(node.children(), &inner);
            }

            // Content only, nothing of the element. `<font>` lands here on
            // purpose: its attributes are the ones refused above.
            _ => self.children(node.children(), &inner),
        }
    }

    fn list(
        &mut self,
        node: &ruma_html::NodeRef,
        ordered: bool,
        element: &ruma_html::ElementData,
        context: &Context,
    ) {
        let mut number: u64 = if ordered {
            attribute(element, "start")
                .and_then(|start| start.parse().ok())
                .unwrap_or(1)
        } else {
            0
        };

        for child in node.children() {
            let is_item = child
                .as_element()
                .map(|data| &*data.name.local == "li")
                .unwrap_or(false);
            if !is_item {
                // Whitespace, or a nested list put directly under the `<ul>`.
                self.node(&child, context);
                continue;
            }

            self.block_break();
            if ordered {
                self.push_tag(&format!("{number}. "));
                number = number.saturating_add(1);
            } else {
                self.push_tag("• ");
            }
            self.children(child.children(), context);
        }
    }

    fn anchor(
        &mut self,
        node: &ruma_html::NodeRef,
        element: &ruma_html::ElementData,
        context: &Context,
    ) {
        let href = attribute(element, "href").unwrap_or_default();
        let usable = !context.in_anchor && is_web_url(&href);

        if !usable {
            // Mention, `matrix:`, `mailto:`: text stays, target goes.
            let inner = Context { depth: context.depth + 1, ..*context };
            self.children(node.children(), &inner);
            return;
        }

        self.push_tag("<a href=\"");
        // Sender's string: escaped like any text, plus the closing quote.
        for character in href.chars() {
            match character {
                '&' => self.push_raw("&amp;"),
                '<' => self.push_raw("&lt;"),
                '>' => self.push_raw("&gt;"),
                '"' => self.push_raw("&quot;"),
                '\'' => self.push_raw("&#39;"),
                _ => {
                    let mut buffer = [0u8; 4];
                    let encoded: &str = character.encode_utf8(&mut buffer);
                    self.push_raw(encoded);
                }
            }
        }
        self.push_tag("\">");

        let inner = Context { in_anchor: true, depth: context.depth + 1, ..*context };
        self.children(node.children(), &inner);
        self.push_tag("</a>");
    }
}

fn attribute(element: &ruma_html::ElementData, name: &str) -> Option<String> {
    element
        .attrs
        .borrow()
        .iter()
        .find(|attribute| &*attribute.name.local == name)
        .map(|attribute| attribute.value.to_string())
}

/// Only http(s) with a host. An anchor is a tap that hands the URL to whatever
/// application claims the scheme, so nothing else keeps its target.
///
/// Characters that cannot occur in a URL are rejected rather than escaped: the
/// parser unescapes the attribute before this sees it, and a target that needs
/// escaping to stay in its own attribute is not one to hand on.
fn is_web_url(href: &str) -> bool {
    if href
        .chars()
        .any(|character| matches!(character, '"' | '\'' | '<' | '>' | '`') || character.is_control())
    {
        return false;
    }
    let rest = match href
        .strip_prefix("https://")
        .or_else(|| href.strip_prefix("http://"))
    {
        Some(rest) => rest,
        None => return false,
    };
    let host_end = rest
        .find(|character| character == '/' || character == '?' || character == '#')
        .unwrap_or(rest.len());
    let host = &rest[..host_end];
    !host.is_empty() && !host.contains(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_html_stays_on_the_plain_path() {
        // The common case by a wide margin: a client attaches a formatted body
        // to every message, formatted or not.
        assert_eq!(to_styled_text("hello there"), None);
        assert_eq!(to_styled_text(""), None);
    }

    #[test]
    fn emphasis_survives() {
        assert_eq!(
            to_styled_text("<em>yes</em> and <strong>no</strong>").as_deref(),
            Some("<i>yes</i> and <b>no</b>")
        );
    }

    #[test]
    fn scripts_and_pictures_never_reach_qt() {
        // The two that must not survive under any circumstance: one executes,
        // the other reports back that the message was displayed.
        let rendered = to_styled_text("<script>alert(1)</script><b>text</b>").unwrap();
        assert!(!rendered.contains("script"), "{rendered}");
        assert!(!rendered.contains("alert"), "{rendered}");

        // A picture alone renders as its alt text and therefore as nothing the
        // plain body did not already say — no markup, so the plain path stays.
        assert_eq!(to_styled_text("<img src=\"http://tracker.invalid/x\" alt=\"cat\">"), None);

        let rendered =
            to_styled_text("<b>look</b> <img src=\"http://tracker.invalid/x\" alt=\"cat\">")
                .unwrap();
        assert!(!rendered.contains("http"), "{rendered}");
        assert_eq!(rendered, "<b>look</b> cat");
    }

    #[test]
    fn text_that_looks_like_markup_is_escaped() {
        let rendered = to_styled_text("<b>a &lt; b &amp;&amp; c &gt; d</b>").unwrap();
        assert_eq!(rendered, "<b>a &lt; b &amp;&amp; c &gt; d</b>");
    }

    #[test]
    fn only_web_links_keep_their_target() {
        assert_eq!(
            to_styled_text("<a href=\"https://example.invalid/x\">site</a>").as_deref(),
            Some("<a href=\"https://example.invalid/x\">site</a>")
        );
        // A mention keeps its name and loses the URI: tapping it would hand the
        // scheme's owner an argument the sender chose.
        assert_eq!(
            to_styled_text("<b>hi</b> <a href=\"https://matrix.to/#/@x:y\">Name</a>")
                .as_deref(),
            Some("<b>hi</b> <a href=\"https://matrix.to/#/@x:y\">Name</a>")
        );
        let rendered = to_styled_text("<b>x</b><a href=\"javascript:alert(1)\">tap</a>").unwrap();
        assert!(!rendered.contains("javascript"), "{rendered}");
        assert!(rendered.ends_with("tap"), "{rendered}");
        let rendered = to_styled_text("<b>x</b><a href=\"mailto:a@b.invalid\">mail</a>").unwrap();
        assert!(!rendered.contains("mailto"), "{rendered}");
    }

    #[test]
    fn a_target_that_would_need_escaping_is_dropped() {
        // The parser unescapes the attribute before this code sees it, so the
        // quote is real by then. Escaping it again would keep the anchor inside
        // its own tag, but the result is a URL nobody wrote on purpose — it is
        // refused instead, and the link's text stays.
        let rendered =
            to_styled_text("<b>x</b><a href='https://a.invalid/\"><img src=x onerror=y>'>t</a>")
                .unwrap();
        assert!(!rendered.contains("<a "), "{rendered}");
        assert!(!rendered.contains("onerror"), "{rendered}");
        assert!(rendered.ends_with("t"), "{rendered}");
    }

    #[test]
    fn colours_are_dropped() {
        // A sender who may choose the colour may choose the background's.
        let rendered =
            to_styled_text("<font color=\"#101010\"><b>invisible?</b></font>").unwrap();
        assert!(!rendered.contains("color"), "{rendered}");
        assert_eq!(rendered, "<b>invisible?</b>");
    }

    #[test]
    fn lists_become_bullets_and_numbers() {
        assert_eq!(
            to_styled_text("<ul><li>one</li><li>two</li></ul>").as_deref(),
            Some("• one<br>• two")
        );
        assert_eq!(
            to_styled_text("<ol start=\"3\"><li>a</li><li>b</li></ol>").as_deref(),
            Some("3. a<br>4. b")
        );
    }

    #[test]
    fn quotes_get_a_bar_on_every_line() {
        assert_eq!(
            to_styled_text("<blockquote>one<br>two</blockquote>").as_deref(),
            Some("▏ one<br>▏ two")
        );
    }

    #[test]
    fn code_keeps_its_line_breaks_and_indentation() {
        let rendered = to_styled_text("<pre><code>fn a() {\n    b()\n}</code></pre>").unwrap();
        assert!(rendered.contains("<br>&nbsp;&nbsp;&nbsp;&nbsp;b()"), "{rendered}");
        assert!(rendered.contains("monospace"), "{rendered}");
    }

    #[test]
    fn the_reply_fallback_is_not_quoted_twice() {
        // The client draws the quoted message from the event's relation. Left
        // in, the fallback would repeat it — and it is someone else's text.
        let rendered = to_styled_text(
            "<mx-reply><blockquote>old text</blockquote></mx-reply><b>answer</b>",
        )
        .unwrap();
        assert!(!rendered.contains("old text"), "{rendered}");
        assert_eq!(rendered, "<b>answer</b>");
    }

    #[test]
    fn a_spoiler_is_marked() {
        let rendered = to_styled_text("<span data-mx-spoiler>ending</span>").unwrap();
        assert_eq!(rendered, "▨ ending");
    }

    #[test]
    fn oversized_input_is_refused() {
        let huge = format!("<b>{}</b>", "x".repeat(MAX_INPUT));
        assert_eq!(to_styled_text(&huge), None);
    }

    #[test]
    fn deep_nesting_terminates() {
        let deep = format!("{}deep{}", "<b>".repeat(300), "</b>".repeat(300));
        // Whatever comes out, it comes out: no stack overflow, no hang.
        let _ = to_styled_text(&deep);
    }
}
