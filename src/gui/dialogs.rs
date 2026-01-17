use btsms::bluetooth::{BluetoothDevice, DeviceManager};
use gtk4::prelude::*;
use gtk4::{
    ApplicationWindow, Box as GtkBox, Button, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow, SelectionMode,
};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub enum PhoneSelectionResult {
    Selected(BluetoothDevice),
    NoneFound,
    Cancelled,
    Error(String),
}

pub fn show_pairing_instructions(window: &ApplicationWindow) {
    #[allow(deprecated)]
    {
        let dialog = gtk4::MessageDialog::builder()
            .transient_for(window)
            .modal(true)
            .message_type(gtk4::MessageType::Info)
            .buttons(gtk4::ButtonsType::Ok)
            .text("No Paired Phone Found")
            .secondary_text(
                "To pair your phone:\n\n\
                1. Open terminal: bluetoothctl\n\
                2. Type: scan on\n\
                3. Type: pair [MAC_ADDRESS]\n\
                4. Type: trust [MAC_ADDRESS]\n\n\
                For iPhone: Enable 'Show Notifications' in Bluetooth settings",
            )
            .build();
        dialog.present();
    }
}

pub fn show_error_dialog_with_copy(window: &ApplicationWindow, title: &str, message: &str) {
    let dialog = adw::Window::builder()
        .transient_for(window)
        .modal(true)
        .default_width(500)
        .default_height(300)
        .title(title)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content_box = GtkBox::new(Orientation::Vertical, 12);
    content_box.set_margin_start(12);
    content_box.set_margin_end(12);
    content_box.set_margin_top(12);
    content_box.set_margin_bottom(12);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let text_view = gtk4::TextView::builder()
        .editable(false)
        .wrap_mode(gtk4::WrapMode::WordChar)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();

    text_view.buffer().set_text(message);
    scroll.set_child(Some(&text_view));
    content_box.append(&scroll);

    let button_box = GtkBox::new(Orientation::Horizontal, 6);
    button_box.set_halign(gtk4::Align::End);

    let copy_btn = Button::with_label("Copy");
    let ok_btn = Button::with_label("OK");
    ok_btn.add_css_class("suggested-action");

    button_box.append(&copy_btn);
    button_box.append(&ok_btn);
    content_box.append(&button_box);

    toolbar_view.set_content(Some(&content_box));
    dialog.set_content(Some(&toolbar_view));

    let message_clone = message.to_string();
    copy_btn.connect_clicked(move |_| {
        if let Some(display) = gtk4::gdk::Display::default() {
            let clipboard = display.clipboard();
            clipboard.set_text(&message_clone);
        }
    });

    let dialog_clone = dialog.clone();
    ok_btn.connect_clicked(move |_| {
        dialog_clone.close();
    });

    dialog.present();
}

pub async fn select_paired_device(window: &ApplicationWindow) -> PhoneSelectionResult {
    let manager = match DeviceManager::new().await {
        Ok(m) => m,
        Err(e) => return PhoneSelectionResult::Error(format!("Device manager error: {}", e)),
    };

    let phones = match manager.get_all_paired_phones().await {
        Ok(p) => p,
        Err(e) => return PhoneSelectionResult::Error(format!("Failed to get devices: {}", e)),
    };

    if phones.is_empty() {
        return PhoneSelectionResult::NoneFound;
    }

    if phones.len() == 1 {
        let device = phones.into_iter().next().unwrap();
        return connect_and_return_device(&manager, device).await;
    }

    show_phone_selection_dialog(window, phones, manager).await
}

async fn connect_and_return_device(
    manager: &DeviceManager,
    device: BluetoothDevice,
) -> PhoneSelectionResult {
    if !device.connected {
        if let Err(e) = manager.connect_device(&device.address).await {
            return PhoneSelectionResult::Error(format!("Failed to connect: {}", e));
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
    PhoneSelectionResult::Selected(device)
}

async fn show_phone_selection_dialog(
    window: &ApplicationWindow,
    phones: Vec<BluetoothDevice>,
    manager: DeviceManager,
) -> PhoneSelectionResult {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<BluetoothDevice>>();
    let tx = Rc::new(RefCell::new(Some(tx)));

    let dialog = adw::Window::builder()
        .transient_for(window)
        .modal(true)
        .default_width(450)
        .default_height(400)
        .title("Select Phone")
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content_box = GtkBox::new(Orientation::Vertical, 12);
    content_box.set_margin_start(12);
    content_box.set_margin_end(12);
    content_box.set_margin_top(12);
    content_box.set_margin_bottom(12);

    let label = Label::new(Some("Select a phone to connect:"));
    label.set_halign(gtk4::Align::Start);
    content_box.append(&label);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let list_box = ListBox::new();
    list_box.set_selection_mode(SelectionMode::Single);
    list_box.add_css_class("boxed-list");

    let phones_rc = Rc::new(phones);

    for (idx, phone) in phones_rc.iter().enumerate() {
        let row = ListBoxRow::new();
        let row_box = GtkBox::new(Orientation::Vertical, 4);
        row_box.set_margin_start(12);
        row_box.set_margin_end(12);
        row_box.set_margin_top(8);
        row_box.set_margin_bottom(8);

        let name_label = Label::new(Some(&phone.name));
        name_label.set_halign(gtk4::Align::Start);
        name_label.add_css_class("heading");

        let status = if phone.connected {
            "Connected"
        } else {
            "Not connected"
        };
        let detail_label = Label::new(Some(&format!("{} - {}", phone.address, status)));
        detail_label.set_halign(gtk4::Align::Start);
        detail_label.add_css_class("dim-label");

        row_box.append(&name_label);
        row_box.append(&detail_label);
        row.set_child(Some(&row_box));
        row.set_widget_name(&idx.to_string());

        list_box.append(&row);
    }

    if let Some(first_row) = list_box.row_at_index(0) {
        list_box.select_row(Some(&first_row));
    }

    scrolled.set_child(Some(&list_box));
    content_box.append(&scrolled);

    let button_box = GtkBox::new(Orientation::Horizontal, 6);
    button_box.set_halign(gtk4::Align::End);

    let cancel_btn = Button::with_label("Cancel");
    let select_btn = Button::with_label("Connect");
    select_btn.add_css_class("suggested-action");

    button_box.append(&cancel_btn);
    button_box.append(&select_btn);
    content_box.append(&button_box);

    toolbar_view.set_content(Some(&content_box));
    dialog.set_content(Some(&toolbar_view));

    let tx_cancel = tx.clone();
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        if let Some(sender) = tx_cancel.borrow_mut().take() {
            let _ = sender.send(None);
        }
        dialog_cancel.close();
    });

    let tx_select = tx.clone();
    let dialog_select = dialog.clone();
    let phones_select = phones_rc.clone();
    select_btn.connect_clicked(move |_| {
        let selected = list_box.selected_row().and_then(|row| {
            row.widget_name()
                .parse::<usize>()
                .ok()
                .and_then(|idx| phones_select.get(idx).cloned())
        });

        if let Some(sender) = tx_select.borrow_mut().take() {
            let _ = sender.send(selected);
        }
        dialog_select.close();
    });

    let tx_close = tx;
    dialog.connect_close_request(move |_| {
        if let Some(sender) = tx_close.borrow_mut().take() {
            let _ = sender.send(None);
        }
        gtk4::glib::Propagation::Proceed
    });

    dialog.present();

    match rx.await {
        Ok(Some(device)) => connect_and_return_device(&manager, device).await,
        Ok(None) | Err(_) => PhoneSelectionResult::Cancelled,
    }
}
