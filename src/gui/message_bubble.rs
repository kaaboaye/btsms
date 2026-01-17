use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow};
use regex::Regex;
use std::sync::LazyLock;

static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(https?://[^\s<>\[\]()]+)").expect("Invalid URL regex"));

/// Escapes special XML/Pango markup characters
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Converts URLs in text to Pango anchor tags
fn urls_to_links(escaped_text: &str) -> String {
    URL_REGEX
        .replace_all(escaped_text, r#"<a href="$1">$1</a>"#)
        .to_string()
}

/// Converts plain text to Pango markup with clickable URLs
fn text_to_markup(text: &str) -> String {
    let escaped = escape_markup(text);
    urls_to_links(&escaped)
}

pub fn add_message_bubble(list_box: &ListBox, message: &str, is_outgoing: bool, time: &str) {
    let row = ListBoxRow::new();
    row.set_selectable(false);

    let outer_box = GtkBox::new(Orientation::Horizontal, 0);
    outer_box.set_margin_start(12);
    outer_box.set_margin_end(12);
    outer_box.set_margin_top(4);
    outer_box.set_margin_bottom(4);

    let bubble_box = GtkBox::new(Orientation::Vertical, 4);

    let markup = text_to_markup(message);
    let message_label = Label::new(None);
    message_label.set_markup(&markup);
    message_label.set_wrap(true);
    message_label.set_xalign(0.0);
    message_label.set_max_width_chars(40);
    message_label.set_margin_start(12);
    message_label.set_margin_end(12);
    message_label.set_margin_top(8);
    message_label.set_selectable(true);
    message_label.set_use_markup(true);

    let time_label = Label::new(Some(time));
    time_label.add_css_class("dim-label");
    time_label.add_css_class("caption");
    time_label.set_margin_start(12);
    time_label.set_margin_end(12);
    time_label.set_margin_bottom(8);

    bubble_box.append(&message_label);
    bubble_box.append(&time_label);

    if is_outgoing {
        outer_box.set_halign(gtk4::Align::End);
        bubble_box.add_css_class("card");
        bubble_box.add_css_class("outgoing-bubble");
        time_label.set_halign(gtk4::Align::End);
    } else {
        outer_box.set_halign(gtk4::Align::Start);
        bubble_box.add_css_class("card");
        bubble_box.add_css_class("incoming-bubble");
        time_label.set_halign(gtk4::Align::Start);
    }

    outer_box.append(&bubble_box);
    row.set_child(Some(&outer_box));
    list_box.append(&row);
}

pub fn scroll_to_bottom(scroll: &ScrolledWindow) {
    let adj = scroll.vadjustment();
    gtk4::glib::idle_add_local_once(move || {
        adj.set_value(adj.upper() - adj.page_size());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_markup_ampersand() {
        assert_eq!(escape_markup("Tom & Jerry"), "Tom &amp; Jerry");
    }

    #[test]
    fn test_escape_markup_angle_brackets() {
        assert_eq!(escape_markup("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn test_escape_markup_quotes() {
        assert_eq!(
            escape_markup(r#"He said "hello""#),
            "He said &quot;hello&quot;"
        );
    }

    #[test]
    fn test_escape_markup_apostrophe() {
        assert_eq!(escape_markup("it's"), "it&apos;s");
    }

    #[test]
    fn test_escape_markup_mixed() {
        assert_eq!(
            escape_markup("<b>Tom & Jerry's \"show\"</b>"),
            "&lt;b&gt;Tom &amp; Jerry&apos;s &quot;show&quot;&lt;/b&gt;"
        );
    }

    #[test]
    fn test_urls_to_links_http() {
        assert_eq!(
            urls_to_links("Visit http://example.com for more"),
            r#"Visit <a href="http://example.com">http://example.com</a> for more"#
        );
    }

    #[test]
    fn test_urls_to_links_https() {
        assert_eq!(
            urls_to_links("Check https://example.com/path"),
            r#"Check <a href="https://example.com/path">https://example.com/path</a>"#
        );
    }

    #[test]
    fn test_urls_to_links_multiple() {
        assert_eq!(
            urls_to_links("See https://a.com and https://b.com"),
            r#"See <a href="https://a.com">https://a.com</a> and <a href="https://b.com">https://b.com</a>"#
        );
    }

    #[test]
    fn test_urls_to_links_with_query_params() {
        assert_eq!(
            urls_to_links("Link: https://example.com/search?q=test&amp;page=1"),
            r#"Link: <a href="https://example.com/search?q=test&amp;page=1">https://example.com/search?q=test&amp;page=1</a>"#
        );
    }

    #[test]
    fn test_urls_to_links_no_url() {
        assert_eq!(urls_to_links("Hello world"), "Hello world");
    }

    #[test]
    fn test_text_to_markup_plain_text() {
        assert_eq!(text_to_markup("Hello world"), "Hello world");
    }

    #[test]
    fn test_text_to_markup_with_special_chars_and_url() {
        assert_eq!(
            text_to_markup("Check <this> at https://example.com"),
            r#"Check &lt;this&gt; at <a href="https://example.com">https://example.com</a>"#
        );
    }

    #[test]
    fn test_text_to_markup_url_with_ampersand() {
        assert_eq!(
            text_to_markup("Go to https://example.com?a=1&b=2"),
            r#"Go to <a href="https://example.com?a=1&amp;b=2">https://example.com?a=1&amp;b=2</a>"#
        );
    }

    #[test]
    fn test_text_to_markup_empty() {
        assert_eq!(text_to_markup(""), "");
    }
}
