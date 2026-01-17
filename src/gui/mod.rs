mod chat_view;
mod connection;
mod conversation_row;
mod dialogs;
mod event_handlers;
mod handlers;
mod header_bar;
mod helpers;
mod init;
mod message_bubble;
mod settings;
mod sidebar;
mod state;

use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Box as GtkBox, Orientation, Paned};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Mutex;

use chat_view::build_chat_view;
use event_handlers::{
    setup_conversation_selection_handler, setup_device_switcher_handler, setup_new_message_handler,
    setup_send_handler, setup_settings_handler,
};
use header_bar::build_header_bar;
use init::spawn_initialization;
use sidebar::build_sidebar;
use state::{AppState, UiState};

pub fn build_ui(app: &adw::Application) {
    // Create main window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Bluetooth SMS")
        .default_width(1000)
        .default_height(700)
        .build();

    // Main container with header
    let main_box = GtkBox::new(Orientation::Vertical, 0);

    // Build header bar
    let header_widgets = build_header_bar();
    main_box.append(&header_widgets.header);

    // Main content: Paned layout with sidebar and chat view
    let paned = Paned::new(Orientation::Horizontal);
    paned.set_position(280);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);

    // Build sidebar
    let sidebar_widgets = build_sidebar();
    paned.set_start_child(Some(&sidebar_widgets.container));

    // Build chat view
    let chat_widgets = build_chat_view();
    paned.set_end_child(Some(&chat_widgets.container));

    main_box.append(&paned);
    window.set_child(Some(&main_box));

    // Create shared state
    let app_state = Arc::new(Mutex::new(AppState::new()));
    let ui_state = Rc::new(RefCell::new(UiState {
        conversation_list: sidebar_widgets.conversation_list.clone(),
        message_list: chat_widgets.message_list.clone(),
        recipient_entry: chat_widgets.recipient_entry.clone(),
        message_entry: chat_widgets.message_entry.clone(),
        message_scroll: chat_widgets.message_scroll.clone(),
    }));

    // Setup event handlers
    setup_new_message_handler(
        &sidebar_widgets.new_message_button,
        app_state.clone(),
        ui_state.clone(),
    );

    setup_conversation_selection_handler(
        &sidebar_widgets.conversation_list,
        app_state.clone(),
        ui_state.clone(),
    );

    setup_device_switcher_handler(
        &header_widgets.device_switch_button,
        app_state.clone(),
        ui_state.clone(),
        header_widgets.status_label.clone(),
        chat_widgets.send_button.clone(),
    );

    setup_send_handler(
        &chat_widgets.send_button,
        &chat_widgets.message_entry,
        app_state.clone(),
        ui_state.clone(),
        header_widgets.status_label.clone(),
        window.clone(),
    );

    setup_settings_handler(
        &header_widgets.settings_button,
        app_state.clone(),
        ui_state.clone(),
        window.clone(),
        header_widgets.status_label.clone(),
        header_widgets.device_switch_button.clone(),
        chat_widgets.send_button.clone(),
    );

    // Initialize app (database, auto-connect, load conversations)
    spawn_initialization(
        app_state,
        ui_state,
        header_widgets.status_label,
        chat_widgets.send_button,
        header_widgets.device_switch_button,
        window.clone(),
    );

    window.present();
}
