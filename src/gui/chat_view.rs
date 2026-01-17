use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Entry, Label, ListBox, Orientation, ScrolledWindow, SelectionMode,
};

pub struct ChatViewWidgets {
    pub container: GtkBox,
    pub recipient_entry: Entry,
    pub message_list: ListBox,
    pub message_scroll: ScrolledWindow,
    pub message_entry: Entry,
    pub send_button: Button,
}

pub fn build_chat_view() -> ChatViewWidgets {
    let container = GtkBox::new(Orientation::Vertical, 0);

    // Recipient bar at top
    let recipient_bar = GtkBox::new(Orientation::Horizontal, 8);
    recipient_bar.set_margin_start(12);
    recipient_bar.set_margin_end(12);
    recipient_bar.set_margin_top(8);
    recipient_bar.set_margin_bottom(8);

    let to_label = Label::new(Some("To:"));
    to_label.add_css_class("dim-label");

    let recipient_entry = Entry::builder()
        .placeholder_text("Phone number or contact name")
        .hexpand(true)
        .build();

    recipient_bar.append(&to_label);
    recipient_bar.append(&recipient_entry);
    container.append(&recipient_bar);

    // Separator
    let separator = gtk4::Separator::new(Orientation::Horizontal);
    container.append(&separator);

    // Message list (scrollable)
    let message_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let message_list = ListBox::new();
    message_list.set_selection_mode(SelectionMode::None);
    message_list.add_css_class("boxed-list");

    message_scroll.set_child(Some(&message_list));
    container.append(&message_scroll);

    // Compose bar at bottom
    let compose_bar = GtkBox::new(Orientation::Horizontal, 8);
    compose_bar.set_margin_start(12);
    compose_bar.set_margin_end(12);
    compose_bar.set_margin_top(8);
    compose_bar.set_margin_bottom(12);

    let message_entry = Entry::builder()
        .placeholder_text("Message")
        .hexpand(true)
        .build();
    message_entry.add_css_class("message-input");

    let send_button = Button::with_label("Send");
    send_button.add_css_class("suggested-action");
    send_button.set_sensitive(false);

    compose_bar.append(&message_entry);
    compose_bar.append(&send_button);
    container.append(&compose_bar);

    ChatViewWidgets {
        container,
        recipient_entry,
        message_list,
        message_scroll,
        message_entry,
        send_button,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_view_widgets_struct() {
        // Compile-time check that struct has all expected fields
        let _: fn(ChatViewWidgets) = |w| {
            let _ = w.container;
            let _ = w.recipient_entry;
            let _ = w.message_list;
            let _ = w.message_scroll;
            let _ = w.message_entry;
            let _ = w.send_button;
        };
    }
}
