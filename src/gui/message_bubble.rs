use btsms::reactions::{ReactionMark, chip_text, chip_tooltip};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, ListBoxRow, Orientation};
use regex::Regex;
use std::sync::LazyLock;

static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'\[\]()]+"#).expect("valid URL regex"));

/// Escape text for Pango markup.
pub fn escape_markup(text: &str) -> String {
    gtk4::glib::markup_escape_text(text).to_string()
}

/// Plain text to Pango markup with clickable links. URLs are found in the raw text (not the
/// escaped one) and trailing sentence punctuation is left out of the link.
pub fn text_to_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut last = 0;
    for m in URL_REGEX.find_iter(text) {
        let url = m.as_str().trim_end_matches(['.', ',', '!', '?', ';', ':']);
        let end = m.start() + url.len();
        out.push_str(&escape_markup(&text[last..m.start()]));
        let escaped = escape_markup(url);
        out.push_str(&format!("<a href=\"{escaped}\">{escaped}</a>"));
        last = end;
    }
    out.push_str(&escape_markup(&text[last..]));
    out
}

/// A message bubble and the chip under it that shows reactions.
pub struct Bubble {
    pub row: ListBoxRow,
    reactions: Label,
}

impl Bubble {
    pub fn set_reactions(&self, reactions: &[ReactionMark]) {
        let text = chip_text(reactions);
        self.reactions.set_label(&text);
        let tooltip = chip_tooltip(reactions);
        self.reactions
            .set_tooltip_text(Some(tooltip.as_str()).filter(|t| !t.is_empty()));
        self.reactions.set_visible(!text.is_empty());
    }
}

fn side(is_outgoing: bool) -> gtk4::Align {
    if is_outgoing {
        gtk4::Align::End
    } else {
        gtk4::Align::Start
    }
}

/// Row holding `content` on the incoming (left) or outgoing (right) side.
fn side_row(content: &impl IsA<gtk4::Widget>, is_outgoing: bool) -> ListBoxRow {
    let outer = GtkBox::new(Orientation::Vertical, 0);
    outer.set_margin_start(if is_outgoing { 48 } else { 12 });
    outer.set_margin_end(if is_outgoing { 12 } else { 48 });
    outer.set_margin_top(2);
    outer.set_margin_bottom(2);
    outer.set_halign(side(is_outgoing));
    outer.append(content);
    ListBoxRow::builder()
        .selectable(false)
        .activatable(false)
        .child(&outer)
        .build()
}

pub fn build_message_bubble(
    message: &str,
    is_outgoing: bool,
    time: &str,
    reactions: &[ReactionMark],
) -> Bubble {
    bubble_with(message, is_outgoing, time, reactions, None)
}

/// A message that is not stored, rebuilt from the quote in a reaction (`reacted_at` is the
/// reaction's time; the message's own time is unknown).
pub fn build_quoted_bubble(
    text: &str,
    is_outgoing: bool,
    reacted_at: &str,
    reactions: &[ReactionMark],
) -> Bubble {
    let bubble = bubble_with(
        text,
        is_outgoing,
        "Text from a reaction",
        reactions,
        Some(&format!(
            "This message is not stored (for example, it was sent from the iPhone itself). \
             Its text comes from a reaction to it received at {reacted_at}."
        )),
    );
    bubble.row.add_css_class("quoted");
    bubble
}

fn bubble_with(
    message: &str,
    is_outgoing: bool,
    time: &str,
    reactions: &[ReactionMark],
    tooltip: Option<&str>,
) -> Bubble {
    let bubble = GtkBox::new(Orientation::Vertical, 2);
    bubble.add_css_class("bubble");
    bubble.add_css_class(if is_outgoing { "outgoing" } else { "incoming" });
    bubble.set_tooltip_text(tooltip);

    let text = Label::builder()
        .use_markup(true)
        .label(text_to_markup(message))
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .xalign(0.0)
        .max_width_chars(45)
        .selectable(true)
        .build();
    let time_label = Label::builder()
        .label(time)
        .css_classes(["bubble-time"])
        .halign(side(is_outgoing))
        .build();
    bubble.append(&text);
    bubble.append(&time_label);

    // The chip overlaps the bubble's bottom edge (negative margin in CSS), like in Google
    // Messages.
    let chip = Label::builder()
        .css_classes(["reaction-chip"])
        .halign(side(is_outgoing))
        .visible(false)
        .build();
    let column = GtkBox::new(Orientation::Vertical, 0);
    column.append(&bubble);
    column.append(&chip);

    let bubble = Bubble {
        row: side_row(&column, is_outgoing),
        reactions: chip,
    };
    bubble.set_reactions(reactions);
    bubble
}

/// A reaction whose message is not loaded: a small note on the reactor's side.
pub fn build_reaction_note(text: &str, is_outgoing: bool, time: &str) -> ListBoxRow {
    let label = Label::builder()
        .label(format!("{text} · {time}"))
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .xalign(if is_outgoing { 1.0 } else { 0.0 })
        .max_width_chars(45)
        .css_classes(["reaction-note"])
        .build();
    side_row(&label, is_outgoing)
}

pub fn build_day_separator(text: &str) -> ListBoxRow {
    let label = Label::builder()
        .label(text)
        .css_classes(["day-separator"])
        .halign(gtk4::Align::Center)
        .build();
    ListBoxRow::builder()
        .selectable(false)
        .activatable(false)
        .child(&label)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_escaped() {
        assert_eq!(text_to_markup("Tom & Jerry"), "Tom &amp; Jerry");
        assert_eq!(text_to_markup("<script>"), "&lt;script&gt;");
        assert_eq!(text_to_markup(""), "");
        assert_eq!(text_to_markup("Hello world"), "Hello world");
    }

    #[test]
    fn urls_become_links() {
        assert_eq!(
            text_to_markup("Visit http://example.com for more"),
            r#"Visit <a href="http://example.com">http://example.com</a> for more"#
        );
        assert_eq!(
            text_to_markup("See https://a.com and https://b.com/x"),
            r#"See <a href="https://a.com">https://a.com</a> and <a href="https://b.com/x">https://b.com/x</a>"#
        );
    }

    #[test]
    fn ampersands_in_urls_are_escaped_once() {
        assert_eq!(
            text_to_markup("Go to https://example.com?a=1&b=2"),
            r#"Go to <a href="https://example.com?a=1&amp;b=2">https://example.com?a=1&amp;b=2</a>"#
        );
    }

    #[test]
    fn quotes_and_punctuation_are_not_part_of_links() {
        // Old code linked `https://x.pl&quot;` and `https://x.pl.`
        assert_eq!(
            text_to_markup(r#"He said "https://x.pl" ok"#),
            r#"He said &quot;<a href="https://x.pl">https://x.pl</a>&quot; ok"#
        );
        assert_eq!(
            text_to_markup("Check https://x.pl/a."),
            r#"Check <a href="https://x.pl/a">https://x.pl/a</a>."#
        );
    }

    #[test]
    fn markup_injection_is_neutralised() {
        let m = text_to_markup("<b>bold</b> https://x.pl/<i>");
        assert!(!m.contains("<b>"));
        assert!(m.contains("&lt;b&gt;"));
        assert!(m.contains(r#"<a href="https://x.pl/">"#));
    }
}
