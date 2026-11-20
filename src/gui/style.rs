//! Application CSS. Uses libadwaita's named colors so light/dark and accent colors follow
//! the system.

pub const CSS: &str = r#"
.message-list,
.message-list > row {
    background: none;
}
.message-list > row {
    padding: 0;
}
.bubble {
    border-radius: 18px;
    padding: 7px 12px;
}
.bubble.incoming {
    background-color: @card_bg_color;
    color: @card_fg_color;
    box-shadow: 0 0 0 1px alpha(currentColor, 0.08);
}
.bubble.outgoing {
    background-color: @accent_bg_color;
    color: @accent_fg_color;
}
.bubble.outgoing label link,
.bubble.outgoing label link:visited {
    color: @accent_fg_color;
}
.bubble-time {
    font-size: 0.8em;
    opacity: 0.7;
}
.day-separator {
    font-size: 0.85em;
    font-weight: bold;
    opacity: 0.6;
    margin-top: 12px;
    margin-bottom: 4px;
}
.compose-box {
    border-radius: 18px;
    padding: 6px 12px;
    background-color: @view_bg_color;
    box-shadow: 0 0 0 1px alpha(currentColor, 0.15);
}
.compose-box textview,
.compose-box textview text {
    background: none;
}
.unread-badge {
    border-radius: 999px;
    min-width: 20px;
    padding: 0 6px;
    font-size: 0.8em;
    font-weight: bold;
    background-color: @accent_bg_color;
    color: @accent_fg_color;
}
.conversation-unread .conversation-title,
.conversation-unread .conversation-preview {
    font-weight: bold;
}
"#;

pub fn load() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
