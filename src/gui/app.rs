//! Main-window controller. All state lives on the GTK thread; Bluetooth and database work
//! runs on Tokio through [`runtime::run`].

use crate::gui::conversation_row::{
    build_conversation_row, format_bubble_time, format_day, local_day,
};
use crate::gui::message_bubble::{
    Bubble, build_day_separator, build_message_bubble, build_quoted_bubble, build_reaction_note,
};
use crate::gui::window::{self, Widgets, compose_text};
use crate::gui::{dialogs, runtime};
use btsms::bluetooth::{BluetoothDevice, DeviceManager};
use btsms::config::Config;
use btsms::contacts::{ContactManager, normalize_for_sending};
use btsms::db::{self, Conversation, Message, MessageDirection};
use btsms::error::BtsmsError;
use btsms::reactions::{self, ThreadItem};
use btsms::service::{self, LinkEvent, LinkSettings, LinkState, PhoneLink};
use chrono::NaiveDate;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use sqlx::SqlitePool;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// Messages loaded when a conversation is opened.
const CONVERSATION_PAGE: i64 = 300;
/// More new messages than this at once get a single summary notification.
const MAX_SEPARATE_NOTIFICATIONS: usize = 3;

pub struct App {
    pub application: adw::Application,
    pub w: Widgets,
    pool: RefCell<Option<SqlitePool>>,
    pub config: RefCell<Config>,
    link: RefCell<Option<PhoneLink>>,
    /// Bumped on every link start so events from a replaced link are ignored.
    link_generation: Cell<u64>,
    link_state: RefCell<Option<LinkState>>,
    current_peer: RefCell<Option<String>>,
    conversations: RefCell<Vec<Conversation>>,
    last_day: Cell<Option<NaiveDate>>,
    /// Messages of the open conversation, in display order.
    chat_messages: RefCell<Vec<Message>>,
    /// Rows already shown, by message id: the bubble, or `None` for a reaction note.
    shown: RefCell<HashMap<i64, Option<Bubble>>>,
    suppress_selection: Cell<bool>,
    stick_to_bottom: Rc<Cell<bool>>,
    sending: Cell<bool>,
    last_sync_error: RefCell<Option<String>>,
    /// Contact picked from the recipient suggestions: (shown text, number).
    picked_recipient: RefCell<Option<(String, String)>>,
    /// Rows currently shown under the recipient field: (shown text, number).
    suggestions: RefCell<Vec<(String, String)>>,
}

impl App {
    pub fn new(application: &adw::Application, config: Config) -> Rc<Self> {
        let w = window::build(application);
        let app = Rc::new(Self {
            application: application.clone(),
            w,
            pool: RefCell::new(None),
            config: RefCell::new(config),
            link: RefCell::new(None),
            link_generation: Cell::new(0),
            link_state: RefCell::new(None),
            current_peer: RefCell::new(None),
            conversations: RefCell::new(Vec::new()),
            last_day: Cell::new(None),
            chat_messages: RefCell::new(Vec::new()),
            shown: RefCell::new(HashMap::new()),
            suppress_selection: Cell::new(false),
            stick_to_bottom: Rc::new(Cell::new(true)),
            sending: Cell::new(false),
            last_sync_error: RefCell::new(None),
            picked_recipient: RefCell::new(None),
            suggestions: RefCell::new(Vec::new()),
        });
        window::keep_scrolled_to_bottom(&app.w.message_scroll, app.stick_to_bottom.clone());
        Self::connect_signals(&app);
        app
    }

    fn connect_signals(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        // row-activated (not row-selected): in the narrow layout the user goes back to the
        // list and taps the already selected conversation again.
        app.w
            .conversation_list
            .connect_row_activated(move |_, row| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                if app.suppress_selection.get() {
                    return;
                }
                let peer = app
                    .conversations
                    .borrow()
                    .get(row.index() as usize)
                    .map(|c| c.peer.clone());
                if let Some(peer) = peer {
                    glib::spawn_future_local(async move { app.open_conversation(&peer).await });
                }
            });

        let weak = Rc::downgrade(app);
        app.w.conversation_list.set_filter_func(move |row| {
            let Some(app) = weak.upgrade() else {
                return true;
            };
            let query = app.w.search.text().to_lowercase();
            if query.is_empty() {
                return true;
            }
            app.conversations
                .borrow()
                .get(row.index() as usize)
                .is_some_and(|c| {
                    c.title().to_lowercase().contains(&query)
                        || c.peer.contains(&query)
                        || c.last_message.to_lowercase().contains(&query)
                })
        });
        let weak = Rc::downgrade(app);
        app.w.search.connect_search_changed(move |_| {
            if let Some(app) = weak.upgrade() {
                app.w.conversation_list.invalidate_filter();
            }
        });

        // Enter sends, Shift+Enter inserts a newline.
        let key = gtk4::EventControllerKey::new();
        let weak = Rc::downgrade(app);
        key.connect_key_pressed(move |_, keyval, _, modifiers| {
            let is_enter = matches!(keyval, gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter);
            if is_enter && !modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
                if let Some(app) = weak.upgrade() {
                    glib::spawn_future_local(async move { app.send().await });
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        app.w.compose_view.add_controller(key);

        let weak = Rc::downgrade(app);
        app.w.compose_view.buffer().connect_changed(move |_| {
            if let Some(app) = weak.upgrade() {
                app.update_send_sensitivity();
            }
        });
        let weak = Rc::downgrade(app);
        app.w.recipient_entry.connect_changed(move |_| {
            if let Some(app) = weak.upgrade() {
                app.update_send_sensitivity();
                glib::spawn_future_local(async move { app.suggest_recipients().await });
            }
        });
        let weak = Rc::downgrade(app);
        app.w
            .recipient_suggestions
            .connect_row_activated(move |_, row| {
                let Some(app) = weak.upgrade() else { return };
                let Some((label, number)) =
                    app.suggestions.borrow().get(row.index() as usize).cloned()
                else {
                    return;
                };
                *app.picked_recipient.borrow_mut() = Some((label.clone(), number));
                app.w.recipient_entry.set_text(&label);
                app.w.recipient_suggestions.set_visible(false);
                app.w.compose_view.grab_focus();
            });
        let weak = Rc::downgrade(app);
        app.w.recipient_entry.connect_activate(move |_| {
            if let Some(app) = weak.upgrade() {
                app.w.recipient_suggestions.set_visible(false);
                app.w.compose_view.grab_focus();
            }
        });
        let weak = Rc::downgrade(app);
        app.w.send_button.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                glib::spawn_future_local(async move { app.send().await });
            }
        });

        let weak = Rc::downgrade(app);
        app.w.window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                return;
            }
            let Some(app) = weak.upgrade() else { return };
            let peer = app.current_peer.borrow().clone();
            if let Some(peer) = peer {
                glib::spawn_future_local(async move { app.mark_read(&peer).await });
            }
        });
    }

    pub fn weak(self: &Rc<Self>) -> Weak<Self> {
        Rc::downgrade(self)
    }

    pub fn pool(&self) -> Option<SqlitePool> {
        self.pool.borrow().clone()
    }

    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::builder()
            .title(glib::markup_escape_text(text))
            .timeout(4)
            .build();
        self.w.toasts.add_toast(toast);
    }

    pub fn error(&self, heading: &str, err: &BtsmsError) {
        dialogs::show_error(&self.w.window, heading, &err.to_string());
    }

    pub fn current_device(&self) -> Option<BluetoothDevice> {
        self.link.borrow().as_ref().map(|l| l.device().clone())
    }

    fn is_connected(&self) -> bool {
        matches!(*self.link_state.borrow(), Some(LinkState::Connected { .. }))
    }

    // ---------------------------------------------------------------- startup

    /// Open the database, show conversations, auto-connect.
    pub async fn start(self: Rc<Self>) {
        let config = self.config.borrow().clone();
        match runtime::run(async move { db::open_app_database(&config, true).await }).await {
            Ok(pool) => {
                *self.pool.borrow_mut() = Some(pool);
            }
            Err(e) => {
                let path = self.config.borrow().database_path.display().to_string();
                dialogs::show_error(
                    &self.w.window,
                    "Cannot Open the Message Database",
                    &format!("{e}\n\nDatabase: {path}"),
                );
                return;
            }
        }
        self.refresh_conversations().await;
        self.update_link_ui();
        if self.config.borrow().auto_connect {
            self.auto_connect().await;
        }
    }

    async fn auto_connect(self: &Rc<Self>) {
        let phones =
            match runtime::run(async { DeviceManager::new().await?.get_all_paired_phones().await })
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, "auto-connect: cannot list phones");
                    self.w.banner.set_title("Bluetooth is not available");
                    return;
                }
            };
        let last = self.config.borrow().last_device_address.clone();
        match choose_auto_connect(&phones, last.as_deref()) {
            Some(device) => self.start_link(device),
            None if phones.is_empty() => self.w.banner.set_title("No paired phone found"),
            None => self.w.banner.set_title("Choose which phone to use"),
        }
    }

    // ---------------------------------------------------------------- link

    /// Ask which phone to use and connect to it.
    pub async fn connect_interactive(self: &Rc<Self>) {
        let phones =
            match runtime::run(async { DeviceManager::new().await?.get_all_paired_phones().await })
                .await
            {
                Ok(p) => p,
                Err(e) => return self.error("Bluetooth Is Not Available", &e),
            };
        let current = self.current_device().map(|d| d.address);
        if let Some(device) = dialogs::pick_phone(&self.w.window, &phones, current.as_deref()).await
        {
            self.start_link(device);
        }
    }

    pub fn start_link(self: &Rc<Self>, device: BluetoothDevice) {
        let Some(pool) = self.pool() else { return };
        if let Some(old) = self.link.borrow_mut().take() {
            tracing::info!(device = %old.device().address, "stopping previous link");
            glib::spawn_future_local(old.stop());
        }
        let generation = self.link_generation.get() + 1;
        self.link_generation.set(generation);

        self.config
            .borrow_mut()
            .set_last_device(&device.address, &device.name);
        let settings = LinkSettings::from_config(&self.config.borrow());
        tracing::info!(device = %device.address, name = %device.name, "starting phone link");
        let (link, mut events) = PhoneLink::start(device, pool, settings);
        *self.link.borrow_mut() = Some(link);
        *self.link_state.borrow_mut() = Some(LinkState::Connecting);
        self.update_link_ui();

        let weak = self.weak();
        glib::spawn_future_local(async move {
            while let Some(event) = events.recv().await {
                let Some(app) = weak.upgrade() else { break };
                if app.link_generation.get() != generation {
                    break;
                }
                app.handle_event(event).await;
            }
        });
    }

    pub fn disconnect(self: &Rc<Self>) {
        if let Some(link) = self.link.borrow_mut().take() {
            glib::spawn_future_local(link.stop());
        }
        self.link_generation.set(self.link_generation.get() + 1);
        *self.link_state.borrow_mut() = None;
        self.update_link_ui();
    }

    pub fn sync_now(&self) {
        match self.link.borrow().as_ref() {
            Some(link) => {
                link.sync_now();
                self.toast("Checking for new messages…");
            }
            None => self.toast("No phone connected"),
        }
    }

    /// The phone number region changed: rebuild conversation keys and contact numbers.
    pub async fn renormalize(self: &Rc<Self>) {
        let Some(pool) = self.pool() else { return };
        match runtime::run(async move { db::renormalize(&pool).await }).await {
            Ok(()) => {
                *self.current_peer.borrow_mut() = None;
                self.clear_messages();
                self.w.content_stack.set_visible_child_name("empty");
                self.refresh_conversations().await;
            }
            Err(e) => self.error("Could Not Update Phone Numbers", &e),
        }
    }

    pub fn apply_link_settings(&self) {
        if let Some(link) = self.link.borrow().as_ref() {
            link.update_settings(LinkSettings::from_config(&self.config.borrow()));
        }
    }

    async fn handle_event(self: &Rc<Self>, event: LinkEvent) {
        match event {
            LinkEvent::State(state) => {
                if matches!(state, LinkState::Connected { .. }) {
                    *self.last_sync_error.borrow_mut() = None;
                }
                *self.link_state.borrow_mut() = Some(state);
                self.update_link_ui();
            }
            LinkEvent::NewMessages { messages, initial } => {
                self.on_new_messages(messages, initial).await;
            }
            LinkEvent::SyncFailed(err) => {
                let repeated = self.last_sync_error.borrow().as_deref() == Some(err.as_str());
                if !repeated {
                    self.toast(&format!("Sync problem: {err}"));
                    *self.last_sync_error.borrow_mut() = Some(err);
                }
            }
        }
    }

    fn update_link_ui(&self) {
        let name = self.current_device().map(|d| d.name);
        let state = self.link_state.borrow().clone();
        let (subtitle, banner): (String, Option<(String, Option<&str>)>) = match (&state, &name) {
            (Some(LinkState::Connected { live_notifications }), Some(name)) => (
                if *live_notifications {
                    format!("{name} · live")
                } else {
                    name.clone()
                },
                None,
            ),
            (Some(LinkState::Connecting), Some(name)) => (
                format!("Connecting to {name}…"),
                Some((format!("Connecting to {name}…"), None)),
            ),
            (
                Some(LinkState::Reconnecting {
                    retry_in, reason, ..
                }),
                Some(name),
            ) => (
                format!("{name} · offline"),
                Some((
                    format!(
                        "Can't reach {name} — retrying in {}s ({})",
                        retry_in.as_secs(),
                        short_reason(reason)
                    ),
                    Some("Retry Now"),
                )),
            ),
            _ => (
                "Not connected".into(),
                Some(("No phone connected".into(), Some("Connect"))),
            ),
        };
        self.w.sidebar_title.set_subtitle(&subtitle);
        match banner {
            Some((title, button)) => {
                self.w.banner.set_title(&glib::markup_escape_text(&title));
                self.w.banner.set_button_label(button);
                self.w.banner.set_action_name(Some(
                    if matches!(state, Some(LinkState::Reconnecting { .. })) {
                        "app.sync"
                    } else {
                        "app.connect"
                    },
                ));
                self.w.banner.set_revealed(true);
            }
            None => self.w.banner.set_revealed(false),
        }
        self.update_send_sensitivity();
    }

    // ---------------------------------------------------------------- conversations

    pub async fn refresh_conversations(self: &Rc<Self>) {
        let Some(pool) = self.pool() else { return };
        let conversations =
            match runtime::run(async move { db::get_conversations(&pool).await }).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(error = %e, "loading conversations failed");
                    return;
                }
            };

        self.suppress_selection.set(true);
        let list = &self.w.conversation_list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        // The search filter looks rows up by index in `self.conversations`, and append()
        // runs it immediately, so the new list must be in place first.
        *self.conversations.borrow_mut() = conversations;
        let conversations = self.conversations.borrow().clone();
        let current = self.current_peer.borrow().clone();
        for (i, conv) in conversations.iter().enumerate() {
            list.append(&build_conversation_row(conv));
            if current.as_deref() == Some(conv.peer.as_str())
                && let Some(row) = list.row_at_index(i as i32)
            {
                list.select_row(Some(&row));
            }
        }
        self.w
            .sidebar_stack
            .set_visible_child_name(if conversations.is_empty() {
                "empty"
            } else {
                "list"
            });
        self.suppress_selection.set(false);

        let unread: i64 = self
            .conversations
            .borrow()
            .iter()
            .map(|c| c.unread_count)
            .sum();
        self.w.window.set_title(Some(&if unread > 0 {
            format!("({unread}) BT SMS")
        } else {
            "BT SMS".to_string()
        }));
    }

    pub async fn open_conversation(self: &Rc<Self>, peer: &str) {
        let Some(pool) = self.pool() else { return };
        *self.current_peer.borrow_mut() = Some(peer.to_string());
        self.w.recipient_bar.set_visible(false);
        self.w.recipient_suggestions.set_visible(false);
        self.w.content_stack.set_visible_child_name("chat");
        self.w.split.set_show_content(true);

        let title = self
            .conversations
            .borrow()
            .iter()
            .find(|c| c.peer == peer)
            .map(|c| c.title().to_string())
            .unwrap_or_else(|| peer.to_string());
        self.w.chat_title.set_title(&title);
        self.w
            .chat_title
            .set_subtitle(if title == peer { "" } else { peer });

        let p = peer.to_string();
        let messages = match runtime::run(async move {
            db::get_messages_for_conversation(&pool, &p, CONVERSATION_PAGE).await
        })
        .await
        {
            Ok(m) => m,
            Err(e) => return self.error("Cannot Load Messages", &e),
        };
        // The user may have clicked elsewhere meanwhile.
        if self.current_peer.borrow().as_deref() != Some(peer) {
            return;
        }
        self.clear_messages();
        self.stick_to_bottom.set(true);
        self.show_messages(&messages);
        self.w.compose_view.grab_focus();
        self.application
            .withdraw_notification(&notification_id(peer));
        self.mark_read(peer).await;
    }

    pub fn new_message(&self) {
        *self.current_peer.borrow_mut() = None;
        self.suppress_selection.set(true);
        self.w.conversation_list.unselect_all();
        self.suppress_selection.set(false);
        self.clear_messages();
        self.w.chat_title.set_title("New Message");
        self.w.chat_title.set_subtitle("");
        self.w.recipient_entry.set_text("");
        *self.picked_recipient.borrow_mut() = None;
        self.w.recipient_suggestions.set_visible(false);
        self.w.recipient_bar.set_visible(true);
        self.w.content_stack.set_visible_child_name("chat");
        self.w.split.set_show_content(true);
        self.w.recipient_entry.grab_focus();
        self.update_send_sensitivity();
    }

    fn clear_messages(&self) {
        let list = &self.w.message_list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        self.last_day.set(None);
        self.chat_messages.borrow_mut().clear();
        self.shown.borrow_mut().clear();
    }

    /// Add messages to the open conversation. Reactions are attached to the bubble of the
    /// message they quote, which may already be on screen.
    fn show_messages(&self, new: &[Message]) {
        self.chat_messages.borrow_mut().extend_from_slice(new);
        let messages = self.chat_messages.borrow();
        let mut shown = self.shown.borrow_mut();
        for item in reactions::build_thread(&messages) {
            match item {
                ThreadItem::Message { message, reactions } => {
                    if let Some(Some(bubble)) = shown.get(&message.id) {
                        bubble.set_reactions(&reactions);
                        continue;
                    }
                    self.append_day_separator(&message.received_at);
                    let bubble = build_message_bubble(
                        &message.body,
                        message.direction == MessageDirection::Outgoing,
                        &format_bubble_time(&message.received_at),
                        &reactions,
                    );
                    self.w.message_list.append(&bubble.row);
                    shown.insert(message.id, Some(bubble));
                }
                ThreadItem::Quoted {
                    reaction_message,
                    text,
                    direction,
                    reactions,
                } => {
                    // Keyed by the reaction it was rebuilt from.
                    if let Some(Some(bubble)) = shown.get(&reaction_message.id) {
                        bubble.set_reactions(&reactions);
                        continue;
                    }
                    self.append_day_separator(&reaction_message.received_at);
                    let bubble = build_quoted_bubble(
                        &text,
                        direction == MessageDirection::Outgoing,
                        &format_bubble_time(&reaction_message.received_at),
                        &reactions,
                    );
                    self.w.message_list.append(&bubble.row);
                    shown.insert(reaction_message.id, Some(bubble));
                }
                ThreadItem::Unmatched { message, reaction } => {
                    if shown.contains_key(&message.id) {
                        continue;
                    }
                    self.append_day_separator(&message.received_at);
                    self.w.message_list.append(&build_reaction_note(
                        &reaction.describe(),
                        message.direction == MessageDirection::Outgoing,
                        &format_bubble_time(&message.received_at),
                    ));
                    shown.insert(message.id, None);
                }
            }
        }
    }

    fn append_day_separator(&self, received_at: &str) {
        if let Some(day) = local_day(received_at)
            && self.last_day.get() != Some(day)
        {
            let today = chrono::Local::now().date_naive();
            self.w
                .message_list
                .append(&build_day_separator(&format_day(day, today)));
            self.last_day.set(Some(day));
        }
    }

    async fn mark_read(self: &Rc<Self>, peer: &str) {
        if !self.w.window.is_active() {
            return;
        }
        let Some(pool) = self.pool() else { return };
        let p = peer.to_string();
        if let Ok(changed) =
            runtime::run(async move { db::mark_conversation_read(&pool, &p).await }).await
            && changed > 0
        {
            self.refresh_conversations().await;
        }
    }

    async fn on_new_messages(self: &Rc<Self>, messages: Vec<Message>, initial: bool) {
        let current = self.current_peer.borrow().clone();
        let in_open_chat: Vec<Message> = messages
            .iter()
            .filter(|m| current.as_deref() == Some(m.peer.as_str()))
            .cloned()
            .collect();
        let shown_in_open_chat = !in_open_chat.is_empty();
        if shown_in_open_chat {
            self.show_messages(&in_open_chat);
        }
        self.refresh_conversations().await;
        if shown_in_open_chat && let Some(peer) = &current {
            self.mark_read(peer).await;
        }
        // The first sync after start imports a backlog: it only updates the lists (no toast,
        // which would cover the compose box, and no notifications).
        if initial {
            return;
        }
        let to_notify: Vec<&Message> = messages
            .iter()
            .filter(|m| m.direction == MessageDirection::Incoming && !m.read_status)
            .filter(|m| !(self.w.window.is_active() && current.as_deref() == Some(m.peer.as_str())))
            .collect();
        if to_notify.len() > MAX_SEPARATE_NOTIFICATIONS {
            let notification = gio::Notification::new(&format!("{} new messages", to_notify.len()));
            notification.set_icon(&gio::ThemedIcon::new("btsms"));
            self.application
                .send_notification(Some("new-messages"), &notification);
        } else {
            for m in to_notify {
                self.notify(m);
            }
        }
    }

    fn notify(&self, m: &Message) {
        let title = self
            .conversations
            .borrow()
            .iter()
            .find(|c| c.peer == m.peer)
            .map(|c| c.title().to_string())
            .or_else(|| m.sender_name.clone())
            .unwrap_or_else(|| m.peer.clone());
        let notification = gio::Notification::new(&title);
        notification.set_body(Some(&reactions::display_text(&m.body)));
        notification.set_icon(&gio::ThemedIcon::new("btsms"));
        notification.set_default_action_and_target_value(
            "app.open-conversation",
            Some(&m.peer.to_variant()),
        );
        self.application
            .send_notification(Some(&notification_id(&m.peer)), &notification);
    }

    // ---------------------------------------------------------------- sending

    fn update_send_sensitivity(&self) {
        let has_text = !compose_text(&self.w.compose_view).trim().is_empty();
        let has_recipient = self.current_peer.borrow().is_some()
            || !self.w.recipient_entry.text().trim().is_empty();
        self.w
            .send_button
            .set_sensitive(has_text && has_recipient && self.is_connected() && !self.sending.get());
    }

    /// Show contacts matching what is typed in the recipient field.
    async fn suggest_recipients(self: &Rc<Self>) {
        let typed = self.w.recipient_entry.text().trim().to_string();
        let list = &self.w.recipient_suggestions;
        let picked = self
            .picked_recipient
            .borrow()
            .as_ref()
            .is_some_and(|(label, _)| *label == typed);
        if picked || typed.chars().count() < 2 || self.current_peer.borrow().is_some() {
            if !picked {
                *self.picked_recipient.borrow_mut() = None;
            }
            list.set_visible(false);
            return;
        }
        *self.picked_recipient.borrow_mut() = None;
        let Some(pool) = self.pool() else { return };
        let query = typed.clone();
        let Ok(contacts) =
            runtime::run(async move { ContactManager::new(pool).search(&query).await }).await
        else {
            return;
        };
        // Typing continued while we searched: a newer search will fill the list.
        if self.w.recipient_entry.text().trim() != typed {
            return;
        }
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let mut picked = Vec::new();
        'contacts: for contact in &contacts {
            for phone in &contact.phone_numbers {
                let kind = phone.phone_type.to_lowercase();
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&contact.display_name).as_str())
                    .subtitle(format!("{} · {kind}", phone.normalized))
                    .activatable(true)
                    .build();
                list.append(&row);
                picked.push((
                    format!("{} ({kind})", contact.display_name),
                    phone.original.clone(),
                ));
                if picked.len() == 6 {
                    break 'contacts;
                }
            }
        }
        list.set_visible(!picked.is_empty());
        *self.suggestions.borrow_mut() = picked;
    }

    /// Number to send to. In an existing conversation this is the number exactly as the
    /// phone reported it (not the normalised key, which depends on the region setting).
    async fn resolve_recipient(&self) -> Result<String, BtsmsError> {
        let current = self.current_peer.borrow().clone();
        if let Some(peer) = current {
            let pool = self.pool().ok_or(BtsmsError::NotConnected)?;
            let key = peer.clone();
            let raw = runtime::run(async move { db::reply_address(&pool, &key).await }).await?;
            return Ok(raw.unwrap_or(peer));
        }
        let typed = self.w.recipient_entry.text().trim().to_string();
        let picked = self.picked_recipient.borrow().clone();
        if let Some((label, number)) = picked
            && label == typed
        {
            return Ok(number);
        }
        if typed.chars().any(char::is_alphabetic) {
            let pool = self.pool().ok_or(BtsmsError::NotConnected)?;
            let name = typed.clone();
            return runtime::run(
                async move { ContactManager::new(pool).number_for_name(&name).await },
            )
            .await?
            .ok_or_else(|| BtsmsError::Parse(format!("No contact named '{typed}'")));
        }
        normalize_for_sending(&typed)
    }

    pub async fn send(self: &Rc<Self>) {
        if self.sending.get() {
            return;
        }
        let text = compose_text(&self.w.compose_view);
        if text.trim().is_empty() {
            return;
        }
        let Some(handle) = self.link.borrow().as_ref().map(|l| l.handle()) else {
            return self.toast("Connect a phone first");
        };
        if !self.is_connected() {
            return self.toast("The phone is not connected right now");
        }

        // Guard first: resolving a contact name awaits, and a second Enter must not send twice.
        self.sending.set(true);
        self.update_send_sensitivity();
        self.w.compose_view.set_editable(false);
        let from_peer = self.current_peer.borrow().clone();

        let result = match self.resolve_recipient().await {
            Ok(recipient) => {
                let text_for_send = text.clone();
                runtime::run(async move { handle.send_sms(&recipient, &text_for_send).await }).await
            }
            Err(e) => Err(e),
        };
        self.sending.set(false);
        self.w.compose_view.set_editable(true);

        match result {
            Ok(message) => {
                // The user may have switched conversations while the phone was busy.
                let still_here = *self.current_peer.borrow() == from_peer;
                if still_here {
                    self.w.compose_view.buffer().set_text("");
                }
                let showing = self.current_peer.borrow().clone();
                if showing.as_deref() == Some(message.peer.as_str()) {
                    self.stick_to_bottom.set(true);
                    self.show_messages(std::slice::from_ref(&message));
                    self.refresh_conversations().await;
                } else if still_here && from_peer.is_none() {
                    // Sent from "New Message": continue in that conversation.
                    self.refresh_conversations().await;
                    self.open_conversation(&message.peer).await;
                } else {
                    self.refresh_conversations().await;
                    self.toast("Message sent");
                }
            }
            Err(BtsmsError::Parse(e)) => self.toast(&e),
            Err(e) => self.error("Message Not Sent", &e),
        }
        self.update_send_sensitivity();
        self.w.compose_view.grab_focus();
    }

    // ---------------------------------------------------------------- actions

    pub async fn sync_contacts(self: &Rc<Self>) {
        let (Some(device), Some(pool)) = (self.current_device(), self.pool()) else {
            return self.toast("Connect a phone first");
        };
        self.toast("Downloading contacts…");
        let address = device.address.clone();
        match runtime::run(async move { service::sync_contacts(&address, &pool).await }).await {
            Ok(n) => {
                self.toast(&format!("Synced {n} contacts"));
                self.refresh_conversations().await;
            }
            Err(e) => self.error("Contact Sync Failed", &e),
        }
    }

    pub async fn clear_database(self: &Rc<Self>) {
        let confirmed = dialogs::confirm(
            &self.w.window,
            "Delete All Messages and Contacts?",
            "This only affects this computer; nothing is deleted on the phone. \
             Messages still on the phone are imported again on the next sync.",
            "Delete",
            true,
        )
        .await;
        if !confirmed {
            return;
        }
        let Some(pool) = self.pool() else { return };
        match runtime::run(async move { db::clear_all(&pool).await }).await {
            Ok(()) => {
                *self.current_peer.borrow_mut() = None;
                self.clear_messages();
                self.w.content_stack.set_visible_child_name("empty");
                self.refresh_conversations().await;
                self.toast("Local data deleted");
            }
            Err(e) => self.error("Delete Failed", &e),
        }
    }

    pub fn shutdown(&self) {
        if let Some(link) = self.link.borrow_mut().take() {
            drop(link); // cancels the task; the runtime shutdown waits for it briefly
        }
    }
}

fn notification_id(peer: &str) -> String {
    format!("conversation-{peer}")
}

fn short_reason(reason: &str) -> String {
    let first = reason.split(['.', '\n']).next().unwrap_or(reason).trim();
    crate::gui::conversation_row::truncate_message(first, 60)
}

/// Device to connect to on startup: the last used one if still paired, else the only
/// phone, else the only connected phone. `None` means the user has to choose.
pub fn choose_auto_connect(
    phones: &[BluetoothDevice],
    last_address: Option<&str>,
) -> Option<BluetoothDevice> {
    if let Some(last) = last_address
        && let Some(d) = phones.iter().find(|p| p.address.eq_ignore_ascii_case(last))
    {
        return Some(d.clone());
    }
    if phones.len() == 1 {
        return phones.first().cloned();
    }
    let mut connected = phones.iter().filter(|p| p.connected);
    match (connected.next(), connected.next()) {
        (Some(only), None) => Some(only.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(addr: &str, connected: bool) -> BluetoothDevice {
        BluetoothDevice {
            address: addr.into(),
            name: addr.into(),
            path: String::new(),
            icon: Some("phone".into()),
            class: None,
            paired: true,
            connected,
            trusted: true,
        }
    }

    #[test]
    fn auto_connect_prefers_last_device() {
        let phones = [phone("A", true), phone("B", false)];
        assert_eq!(
            choose_auto_connect(&phones, Some("b")).unwrap().address,
            "B"
        );
    }

    #[test]
    fn auto_connect_single_phone() {
        assert_eq!(
            choose_auto_connect(&[phone("A", false)], None)
                .unwrap()
                .address,
            "A"
        );
        assert_eq!(
            choose_auto_connect(&[phone("A", false)], Some("gone"))
                .unwrap()
                .address,
            "A"
        );
    }

    #[test]
    fn auto_connect_single_connected_among_many() {
        let phones = [phone("A", false), phone("B", true)];
        assert_eq!(choose_auto_connect(&phones, None).unwrap().address, "B");
    }

    #[test]
    fn auto_connect_ambiguous_or_empty() {
        assert!(choose_auto_connect(&[], None).is_none());
        let phones = [phone("A", true), phone("B", true)];
        assert!(choose_auto_connect(&phones, None).is_none());
        let phones = [phone("A", false), phone("B", false)];
        assert!(choose_auto_connect(&phones, None).is_none());
    }

    #[test]
    fn reasons_are_shortened() {
        assert_eq!(short_reason("Host is down. More text"), "Host is down");
        assert_eq!(short_reason(""), "");
    }
}
