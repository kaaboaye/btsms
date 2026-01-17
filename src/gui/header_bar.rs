use gtk4::prelude::*;
use gtk4::{Button, Label};
use libadwaita::HeaderBar;

pub struct HeaderBarWidgets {
    pub header: HeaderBar,
    pub status_label: Label,
    pub reset_button: Button,
    pub connect_button: Button,
    pub sync_button: Button,
    pub import_button: Button,
    pub device_switch_button: Button,
}

pub fn build_header_bar() -> HeaderBarWidgets {
    let header = HeaderBar::new();
    header.set_title_widget(Some(&Label::new(Some("Messages"))));

    let status_label = Label::new(Some("Disconnected"));
    status_label.add_css_class("dim-label");
    header.pack_end(&status_label);

    let reset_button = Button::with_label("Reset DB");
    reset_button.add_css_class("destructive-action");
    header.pack_end(&reset_button);

    let connect_button = Button::with_label("Connect");
    connect_button.add_css_class("suggested-action");
    header.pack_start(&connect_button);

    let sync_button = Button::with_label("Sync Contacts");
    sync_button.set_sensitive(false);
    header.pack_start(&sync_button);

    let import_button = Button::with_label("Import SMS");
    import_button.set_sensitive(false);
    header.pack_start(&import_button);

    let device_switch_button = Button::new();
    device_switch_button.set_icon_name("phone-symbolic");
    device_switch_button.set_tooltip_text(Some("Switch device"));
    device_switch_button.set_visible(false);
    header.pack_start(&device_switch_button);

    HeaderBarWidgets {
        header,
        status_label,
        reset_button,
        connect_button,
        sync_button,
        import_button,
        device_switch_button,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_bar_widgets_struct() {
        // Test that the struct has all expected fields
        // This is a compile-time check - if fields are missing, this won't compile
        let _: fn(HeaderBarWidgets) = |w| {
            let _ = w.header;
            let _ = w.status_label;
            let _ = w.reset_button;
            let _ = w.connect_button;
            let _ = w.sync_button;
            let _ = w.import_button;
            let _ = w.device_switch_button;
        };
    }
}
