use btsms::bluetooth::MapClient;
use btsms::config::Config;
use btsms::contacts::ContactManager;
use gtk4::{Entry, ListBox, ScrolledWindow};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppState {
    pub map_client: Option<MapClient>,
    pub contact_manager: Option<ContactManager>,
    pub db_pool: Option<sqlx::SqlitePool>,
    pub device_address: Option<String>,
    pub device_name: Option<String>,
    pub current_conversation: Option<String>,
    pub config: Config,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            map_client: None,
            contact_manager: None,
            db_pool: None,
            device_address: None,
            device_name: None,
            current_conversation: None,
            config: Config::load(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared UI state that can be accessed from callbacks
pub struct UiState {
    pub conversation_list: ListBox,
    pub message_list: ListBox,
    pub recipient_entry: Entry,
    pub message_entry: Entry,
    pub message_scroll: ScrolledWindow,
}

pub type SharedAppState = Arc<Mutex<AppState>>;
pub type SharedUiState = Rc<RefCell<UiState>>;
