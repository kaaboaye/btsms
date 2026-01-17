use crate::gui::connection::{
    check_obexd_service, complete_connection_setup, connect_to_device,
    determine_auto_connect_device, AutoConnectResult, ConnectResult,
};
use crate::gui::dialogs::show_error_dialog_with_copy;
use crate::gui::handlers::load_conversations;
use crate::gui::state::{SharedAppState, SharedUiState};
use btsms::contacts::ContactManager;
use btsms::db;
use gtk4::glib;
use gtk4::{ApplicationWindow, Button, Label};

/// Initializes the application: database, auto-connect, and loads conversations.
/// This is spawned as a future from build_ui.
pub fn spawn_initialization(
    app_state: SharedAppState,
    ui_state: SharedUiState,
    status_label: Label,
    send_button: Button,
    device_switch_button: Button,
    window: ApplicationWindow,
) {
    glib::spawn_future_local(async move {
        initialize_app(
            app_state,
            ui_state,
            status_label,
            send_button,
            device_switch_button,
            window,
        )
        .await;
    });
}

async fn initialize_app(
    app_state: SharedAppState,
    ui_state: SharedUiState,
    status_label: Label,
    send_button: Button,
    device_switch_button: Button,
    window: ApplicationWindow,
) {
    let db_path = db::default_database_path();
    eprintln!("Database path: {:?}", db_path);

    if let Err(e) = std::fs::create_dir_all(db_path.parent().unwrap()) {
        eprintln!("Failed to create database directory: {}", e);
    }

    match db::init_database(db_path.to_str().unwrap()).await {
        Ok(pool) => {
            let contact_manager = ContactManager::new(pool.clone());
            {
                let mut state = app_state.lock().await;
                state.db_pool = Some(pool.clone());
                state.contact_manager = Some(contact_manager);
            }

            let obexd_available = match check_obexd_service().await {
                Ok(true) => {
                    status_label.set_text("Ready");
                    true
                }
                Ok(false) | Err(_) => {
                    status_label.set_text("obexd not running");
                    false
                }
            };

            load_conversations(pool, ui_state.clone()).await;

            if obexd_available {
                attempt_auto_connect(
                    app_state,
                    ui_state,
                    status_label,
                    send_button,
                    device_switch_button,
                )
                .await;
            }
        }
        Err(e) => {
            let error_msg = format!(
                "Failed to initialize database:\n\n{}\n\n\
                Database path: {:?}\n\n\
                This usually means:\n\
                - Parent directory doesn't exist\n\
                - No write permissions\n\
                - Disk is full",
                e, db_path
            );
            eprintln!("{}", error_msg);
            status_label.set_text("Database error");
            show_error_dialog_with_copy(&window, "Database Error", &error_msg);
        }
    }
}

async fn attempt_auto_connect(
    app_state: SharedAppState,
    ui_state: SharedUiState,
    status_label: Label,
    send_button: Button,
    device_switch_button: Button,
) {
    let config = {
        let state = app_state.lock().await;
        state.config.clone()
    };

    if !config.auto_connect {
        return;
    }

    status_label.set_text("Auto-connecting...");

    match determine_auto_connect_device(&config).await {
        AutoConnectResult::Device(device) => {
            match connect_to_device(device, app_state.clone(), &status_label).await {
                ConnectResult::Success { name } => {
                    complete_connection_setup(
                        app_state,
                        ui_state,
                        &send_button,
                        &device_switch_button,
                        &status_label,
                        &name,
                    )
                    .await;
                }
                ConnectResult::Failed(e) => {
                    eprintln!("Auto-connect failed: {}", e);
                    status_label.set_text("Ready (auto-connect failed)");
                }
            }
        }
        AutoConnectResult::MultipleDevices => {
            status_label.set_text("Ready (select device)");
        }
        AutoConnectResult::NoDevices => {
            status_label.set_text("Ready (no devices)");
        }
        AutoConnectResult::Error(e) => {
            eprintln!("Auto-connect error: {}", e);
            status_label.set_text("Ready");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_compiles() {
        // Verify the module compiles correctly
        assert!(true);
    }
}
