use btsms::bluetooth::BluetoothDevice;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

/// Error dialog with a "Copy details" button (handy for bug reports).
pub fn show_error(parent: &impl IsA<gtk4::Widget>, heading: &str, details: &str) {
    tracing::warn!(heading, details, "showing error dialog");
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(details)
        .body_use_markup(false)
        .build();
    dialog.add_response("copy", "Copy Details");
    dialog.add_response("close", "Close");
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    let details = format!("{heading}\n\n{details}");
    dialog.connect_response(Some("copy"), move |_, _| {
        if let Some(display) = gtk4::gdk::Display::default() {
            display.clipboard().set_text(&details);
        }
    });
    dialog.present(Some(parent));
}

/// Ask a yes/no question; resolves to true when `confirm_label` was chosen.
pub async fn confirm(
    parent: &impl IsA<gtk4::Widget>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    destructive: bool,
) -> bool {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance(
        "confirm",
        if destructive {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.choose_future(Some(parent)).await == "confirm"
}

/// Let the user choose one of the paired phones.
pub async fn pick_phone(
    parent: &impl IsA<gtk4::Widget>,
    phones: &[BluetoothDevice],
    current: Option<&str>,
) -> Option<BluetoothDevice> {
    if phones.is_empty() {
        show_error(
            parent,
            "No Paired Phone",
            "Pair your phone in the system Bluetooth settings first.\n\n\
             iPhone: after pairing, open Settings > Bluetooth > (i) next to this computer \
             and enable \"Show Notifications\" (and \"Sync Contacts\" for contact names).\n\n\
             Android: accept the message and contact access requests on the phone.",
        );
        return None;
    }

    let list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();
    for phone in phones {
        let status = if phone.connected {
            "Connected"
        } else {
            "Not connected"
        };
        let row = adw::ActionRow::builder()
            .title(gtk4::glib::markup_escape_text(&phone.name).as_str())
            .subtitle(format!("{} · {status}", phone.address))
            .activatable(true)
            .build();
        list.append(&row);
    }
    let initial = phones
        .iter()
        .position(|p| Some(p.address.as_str()) == current)
        .or_else(|| phones.iter().position(|p| p.connected))
        .unwrap_or(0);
    if let Some(row) = list.row_at_index(initial as i32) {
        list.select_row(Some(&row));
    }

    let dialog = adw::AlertDialog::builder()
        .heading("Choose Phone")
        .body("Messages will be sent and received through this phone.")
        .extra_child(&list)
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("connect", "Connect");
    dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("connect"));
    dialog.set_close_response("cancel");

    if dialog.choose_future(Some(parent)).await != "connect" {
        return None;
    }
    list.selected_row()
        .and_then(|row| phones.get(row.index() as usize).cloned())
}
