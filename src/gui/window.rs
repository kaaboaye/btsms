//! Widget tree of the main window.

use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

pub struct Widgets {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub split: adw::NavigationSplitView,
    pub sidebar_title: adw::WindowTitle,
    pub banner: adw::Banner,
    pub search: gtk4::SearchEntry,
    pub conversation_list: gtk4::ListBox,
    pub sidebar_stack: gtk4::Stack,
    pub chat_title: adw::WindowTitle,
    pub content_stack: gtk4::Stack,
    pub recipient_bar: gtk4::Box,
    pub recipient_entry: gtk4::Entry,
    pub recipient_suggestions: gtk4::ListBox,
    pub message_list: gtk4::ListBox,
    pub message_scroll: gtk4::ScrolledWindow,
    pub compose_view: gtk4::TextView,
    pub send_button: gtk4::Button,
}

fn primary_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let phone = gio::Menu::new();
    phone.append(Some("Connect Phone…"), Some("app.connect"));
    phone.append(Some("Check for New Messages"), Some("app.sync"));
    phone.append(Some("Sync Contacts"), Some("app.sync-contacts"));
    phone.append(Some("Disconnect"), Some("app.disconnect"));
    menu.append_section(None, &phone);
    let app = gio::Menu::new();
    app.append(Some("Preferences"), Some("app.preferences"));
    app.append(Some("Open Log Folder"), Some("app.open-logs"));
    app.append(Some("About BT SMS"), Some("app.about"));
    menu.append_section(None, &app);
    menu
}

pub fn build(application: &adw::Application) -> Widgets {
    // ---------- sidebar ----------
    let sidebar_title = adw::WindowTitle::new("Messages", "Not connected");
    let sidebar_header = adw::HeaderBar::builder()
        .title_widget(&sidebar_title)
        .build();
    let new_button = gtk4::Button::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text("New Message (Ctrl+N)")
        .action_name("app.new-message")
        .build();
    sidebar_header.pack_start(&new_button);
    let menu_button = gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&primary_menu())
        .primary(true)
        .tooltip_text("Main Menu")
        .build();
    sidebar_header.pack_end(&menu_button);

    let banner = adw::Banner::builder()
        .title("No phone connected")
        .button_label("Connect")
        .action_name("app.connect")
        .revealed(true)
        .build();

    let search = gtk4::SearchEntry::builder()
        .placeholder_text("Search conversations")
        .margin_start(8)
        .margin_end(8)
        .margin_top(6)
        .margin_bottom(6)
        .build();

    let conversation_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .build();
    let list_scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .child(&conversation_list)
        .build();
    let empty_sidebar = adw::StatusPage::builder()
        .icon_name("mail-unread-symbolic")
        .title("No Conversations")
        .description("Messages from your phone will show up here.")
        .css_classes(["compact"])
        .vexpand(true)
        .build();
    let sidebar_stack = gtk4::Stack::new();
    sidebar_stack.add_named(&list_scroll, Some("list"));
    sidebar_stack.add_named(&empty_sidebar, Some("empty"));
    sidebar_stack.set_visible_child_name("empty");

    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_box.append(&banner);
    sidebar_box.append(&search);
    sidebar_box.append(&sidebar_stack);

    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&sidebar_header);
    sidebar_view.set_content(Some(&sidebar_box));
    let sidebar_page = adw::NavigationPage::builder()
        .title("Messages")
        .tag("sidebar")
        .child(&sidebar_view)
        .build();

    // ---------- content ----------
    let chat_title = adw::WindowTitle::new("", "");
    let content_header = adw::HeaderBar::builder().title_widget(&chat_title).build();

    let recipient_entry = gtk4::Entry::builder()
        .placeholder_text("Phone number or contact name")
        .hexpand(true)
        .input_purpose(gtk4::InputPurpose::Phone)
        .build();
    let to_label = gtk4::Label::builder()
        .label("To:")
        .css_classes(["dim-label"])
        .build();
    let recipient_bar = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(8)
        .margin_bottom(8)
        .visible(false)
        .build();
    recipient_bar.append(&to_label);
    recipient_bar.append(&recipient_entry);
    let recipient_suggestions = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(8)
        .visible(false)
        .build();

    let message_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["message-list"])
        .valign(gtk4::Align::End)
        .build();
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .child(&message_list)
        .build();
    let message_scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();

    let compose_view = gtk4::TextView::builder()
        .wrap_mode(gtk4::WrapMode::WordChar)
        .accepts_tab(false)
        .hexpand(true)
        .top_margin(4)
        .bottom_margin(4)
        .build();
    compose_view.update_property(&[gtk4::accessible::Property::Label("Message")]);
    let compose_scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(140)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .child(&compose_view)
        .css_classes(["compose-box"])
        .build();
    let send_button = gtk4::Button::builder()
        .label("Send")
        .css_classes(["suggested-action", "pill"])
        .valign(gtk4::Align::End)
        .tooltip_text("Send (Enter). Shift+Enter adds a new line.")
        .sensitive(false)
        .build();
    let compose_bar = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(8)
        .margin_bottom(12)
        .build();
    compose_bar.append(&compose_scroll);
    compose_bar.append(&send_button);
    let compose_clamp = adw::Clamp::builder()
        .maximum_size(760)
        .child(&compose_bar)
        .build();

    let chat_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    chat_box.append(&recipient_bar);
    chat_box.append(&recipient_suggestions);
    chat_box.append(&message_scroll);
    chat_box.append(&compose_clamp);

    let empty_content = adw::StatusPage::builder()
        .icon_name("mail-send-receive-symbolic")
        .title("BT SMS")
        .description("Choose a conversation or start a new one.")
        .vexpand(true)
        .build();
    let content_stack = gtk4::Stack::new();
    content_stack.add_named(&empty_content, Some("empty"));
    content_stack.add_named(&chat_box, Some("chat"));
    content_stack.set_visible_child_name("empty");

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&content_header);
    content_view.set_content(Some(&content_stack));
    let content_page = adw::NavigationPage::builder()
        .title("Conversation")
        .tag("content")
        .child(&content_view)
        .build();

    // ---------- window ----------
    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar_page)
        .content(&content_page)
        .min_sidebar_width(280.0)
        .max_sidebar_width(380.0)
        .build();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&split));

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("BT SMS")
        .icon_name("btsms")
        .default_width(1000)
        .default_height(700)
        .width_request(360)
        .height_request(400)
        .content(&toasts)
        .build();

    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        640.0,
        adw::LengthUnit::Sp,
    ));
    narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(narrow);

    Widgets {
        window,
        toasts,
        split,
        sidebar_title,
        banner,
        search,
        conversation_list,
        sidebar_stack,
        chat_title,
        content_stack,
        recipient_bar,
        recipient_entry,
        recipient_suggestions,
        message_list,
        message_scroll,
        compose_view,
        send_button,
    }
}

/// Text currently in the compose box.
pub fn compose_text(view: &gtk4::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

/// Keep `scroll` pinned to the bottom while `stick` is set (new content arriving), and
/// clear `stick` only when the *user* scrolls up to read history.
///
/// GtkAdjustment may emit `value-changed` (clamped value) before `notify::upper` while the
/// list grows, so value changes are only trusted when the geometry did not change.
pub fn keep_scrolled_to_bottom(
    scroll: &gtk4::ScrolledWindow,
    stick: std::rc::Rc<std::cell::Cell<bool>>,
) {
    use std::cell::Cell;
    use std::rc::Rc;

    let adj = scroll.vadjustment();
    let geometry = Rc::new(Cell::new((adj.upper(), adj.page_size())));

    // Scrolling from inside a size-allocate (where upper changes) does not repaint, so the
    // jump to the bottom is deferred to an idle callback.
    let pending = Rc::new(Cell::new(false));
    let follow = {
        let stick = stick.clone();
        let geometry = geometry.clone();
        move |adj: &gtk4::Adjustment| {
            geometry.set((adj.upper(), adj.page_size()));
            if stick.get() && !pending.replace(true) {
                let adj = adj.clone();
                let pending = pending.clone();
                let stick = stick.clone();
                gtk4::glib::idle_add_local_once(move || {
                    pending.set(false);
                    if stick.get() {
                        adj.set_value(adj.upper() - adj.page_size());
                    }
                });
            }
        }
    };
    adj.connect_upper_notify(follow.clone());
    adj.connect_page_size_notify(follow);

    adj.connect_value_changed(move |adj| {
        let (upper, page) = geometry.get();
        if (adj.upper() - upper).abs() > 0.5 || (adj.page_size() - page).abs() > 0.5 {
            return; // layout change, not the user
        }
        stick.set(adj.value() + adj.page_size() >= adj.upper() - 40.0);
    });
}
