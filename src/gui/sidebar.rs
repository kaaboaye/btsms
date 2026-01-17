use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, ListBox, Orientation, ScrolledWindow, SelectionMode};

pub struct SidebarWidgets {
    pub container: GtkBox,
    pub new_message_button: Button,
    pub conversation_list: ListBox,
}

pub fn build_sidebar() -> SidebarWidgets {
    let container = GtkBox::new(Orientation::Vertical, 0);
    container.set_width_request(250);

    // "New Message" button at top of sidebar
    let new_message_button = Button::with_label("New Message");
    new_message_button.set_margin_start(8);
    new_message_button.set_margin_end(8);
    new_message_button.set_margin_top(8);
    new_message_button.set_margin_bottom(8);
    container.append(&new_message_button);

    // Conversation list
    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let conversation_list = ListBox::new();
    conversation_list.set_selection_mode(SelectionMode::Single);
    conversation_list.add_css_class("navigation-sidebar");

    scroll.set_child(Some(&conversation_list));
    container.append(&scroll);

    SidebarWidgets {
        container,
        new_message_button,
        conversation_list,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sidebar_widgets_struct() {
        // Compile-time check that struct has all expected fields
        let _: fn(SidebarWidgets) = |w| {
            let _ = w.container;
            let _ = w.new_message_button;
            let _ = w.conversation_list;
        };
    }
}
