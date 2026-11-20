# How BT SMS talks to the phone

Everything goes through BlueZ over D-Bus; there is no Bluetooth code of our own.

| What | Profile | D-Bus | Notes |
|------|---------|-------|-------|
| Read and send messages | MAP (Message Access) | `org.bluez.obex` on the **session** bus (obexd) | iPhone and Android |
| Contacts | PBAP (Phonebook Access) | `org.bluez.obex`, session bus | iPhone needs "Sync Contacts" |
| Instant "new message" signal | ANCS (Apple Notification Center Service) | `org.bluez` GATT on the **system** bus | iPhone only |
| Pairing, connect, device list | — | `org.bluez` `Device1`, system bus | |

## MAP (obexd)

* `Client1.CreateSession(address, {Target: "map"})` returns a session path. obexd removes the
  session when the D-Bus connection that created it closes, so every `ObexSession` owns a
  private connection and dropping it cleans up.
* Navigate with `MessageAccess1.SetFolder("/")` then `SetFolder("telecom/msg/inbox")`. MAP has
  one current folder per session, so all MAP calls are serialised in `MapClient`.
* `ListMessages("", {MaxCount})` returns `{message_path: properties}`. Properties include
  `Subject` (first ≤256 chars of the text), `Timestamp` (`YYYYMMDDTHHMMSS`, local time),
  `Sender`, `SenderAddress`, `Size` (bytes of the full text), `Read`.
* The full text comes from `org.bluez.obex.Message1.Get("", false)` on the **message object**,
  not from `MessageAccess1` (the old code called a method that does not exist there, so it
  always stored the truncated subject). With an empty target obexd picks a temp file and
  returns it as `Filename`.
* Sync only downloads a message when `Subject` is shorter than `Size`; short texts are
  complete in the listing.
* Sending: write a bMessage (see `src/bluetooth/vmessage.rs`) to a file and call
  `PushMessage(file, "outbox", {})` from `telecom/msg`.

### Transfers

`Get`, `PullAll` and `PushMessage` return a `Transfer1` object. obexd deletes it the moment
the transfer ends, so polling `Status` usually hits `UnknownObject`. `TransferWatcher`
subscribes to `PropertiesChanged` under the session path *before* starting the transfer and
waits for `Status = complete | error`.

### iPhone specifics (checked on iOS, Nov 2026)

* The inbox only lists recent messages; there is no full history.
* `sent`, `outbox` and `deleted` exist but are always empty, so messages you send from the
  iPhone itself never show up. Messages sent from this app are stored locally.
* The originator vCard in a bMessage uses `FN;CHARSET=UTF-8:` and the envelope recipient uses
  `TEL;CHARSET=UTF-8:` — parameters must be handled.
* Everything requires "Show Notifications" in iPhone Settings > Bluetooth > (i).

## PBAP

`PhonebookAccess1.Select("int", "pb")` then `PullAll("", {Format: "vcard30", Fields: [...]})`.
Without the `Fields` filter phones include contact photos: a 460-contact iPhone book was
2.2 MB / 16 s with photos and 62 KB / 1 s without.

## ANCS (iPhone)

Once the iPhone is paired and "Show Notifications" is on, BlueZ exposes its GATT database under
the device object. No scanning or separate BLE library is needed.

| Characteristic | UUID |
|---|---|
| Service | `7905f431-b5ce-4e99-a40f-4b1e122d00d0` |
| Notification Source (notify) | `9fbf120d-6301-42d9-8c58-25e699a21dbd` |
| Control Point (write) | `69d1d8f3-45e1-49a8-9821-9bbdfdaad9d9` |
| Data Source (notify) | `22eac6e9-24d6-4bb5-be44-b36ace7c7bfb` |

* `StartNotify` on Data Source and Notification Source, then watch `PropertiesChanged.Value`.
* On subscribe the phone replays every notification it holds with the *PreExisting* flag;
  those are ignored.
* For a new notification, write `GetNotificationAttributes` (command 0, UID, then attribute
  ids; Title and Message take a 2-byte max length, AppIdentifier and Date do not).
* The Control Point takes **one request at a time**; a second write fails with
  `org.bluez.Error.InProgress`. Requests are queued.
* Replies on Data Source can be split over several notifications and are reassembled.
* Messages (SMS and iMessage) come from app id `com.apple.MobileSMS`. ANCS only says *that* a
  message arrived (the title is a contact name, not a number), so the app reacts by running a
  MAP sync, which yields the real number and full text.
