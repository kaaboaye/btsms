use gtk4::prelude::*;
use gtk4::{Button, Label};
use libadwaita::HeaderBar;

pub struct HeaderBarWidgets {
    pub header: HeaderBar,
    pub status_label: Label,
    pub device_switch_button: Button,
    pub settings_button: Button,
}

pub fn build_header_bar() -> HeaderBarWidgets {
    let header = HeaderBar::new();
    header.set_title_widget(Some(&Label::new(Some("Messages"))));

    let status_label = Label::new(Some("Disconnected"));
    status_label.add_css_class("dim-label");
    header.pack_end(&status_label);

    let settings_button = Button::new();
    settings_button.set_icon_name("emblem-system-symbolic");
    settings_button.set_tooltip_text(Some("Settings"));
    header.pack_end(&settings_button);

    let device_switch_button = Button::new();
    device_switch_button.set_icon_name("phone-symbolic");
    device_switch_button.set_tooltip_text(Some("Switch device"));
    device_switch_button.set_visible(false);
    header.pack_start(&device_switch_button);

    HeaderBarWidgets {
        header,
        status_label,
        device_switch_button,
        settings_button,
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
            let _ = w.device_switch_button;
            let _ = w.settings_button;
        };
    }
}
