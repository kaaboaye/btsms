use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow};

pub fn add_message_bubble(list_box: &ListBox, message: &str, is_outgoing: bool, time: &str) {
    let row = ListBoxRow::new();
    row.set_selectable(false);

    let outer_box = GtkBox::new(Orientation::Horizontal, 0);
    outer_box.set_margin_start(12);
    outer_box.set_margin_end(12);
    outer_box.set_margin_top(4);
    outer_box.set_margin_bottom(4);

    let bubble_box = GtkBox::new(Orientation::Vertical, 2);
    bubble_box.set_margin_start(8);
    bubble_box.set_margin_end(8);
    bubble_box.set_margin_top(6);
    bubble_box.set_margin_bottom(6);

    let message_label = Label::new(Some(message));
    message_label.set_wrap(true);
    message_label.set_xalign(0.0);
    message_label.set_max_width_chars(40);

    let time_label = Label::new(Some(time));
    time_label.add_css_class("dim-label");
    time_label.add_css_class("caption");

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
