# Agent Development Guidelines

BT SMS: GTK4/libadwaita app + CLI that reads and sends SMS through a phone over Bluetooth
(BlueZ D-Bus: obexd MAP/PBAP on the session bus, ANCS GATT on the system bus).
Read `docs/protocols.md` before touching anything in `src/bluetooth/`.

## Checks (same as CI)

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

No warnings from cargo or clippy. Fix them; never silence them with `#[allow]`.

## Tests

- New logic gets unit tests: parsers, protocol state machines, normalisation, SQL queries
  (use `db::test_pool()`), decision helpers.
- Tests must not touch real Bluetooth, D-Bus or the user's data directory. Keep protocol
  logic separate from transport (see `ancs.rs` vs `ancs_client.rs`) so it can be tested.
- No placeholder tests (`assert!(true)`, "it compiles", constructing a struct and asserting its
  fields). A test must be able to fail when behaviour breaks.
- Fixtures use fictional data (`+48 512 345 678`, "Jan Kowalski"), never real numbers or
  message texts.

## Testing on a real phone

- The phone is usually paired; `btsms-cli devices` / `btsms-cli doctor` show its state.
- Point everything at a scratch directory so the user's real database is untouched:
  `XDG_DATA_HOME=/tmp/x/data XDG_STATE_HOME=/tmp/x/state target/debug/btsms-cli …`
- Listing, `doctor`, `messages inbox`, `contacts sync` are read-only on the phone.
  **Sending an SMS is not** — ask the user first, and prefer `messages send --dry-run`.
- For the GUI, run it on a virtual display (`Xvfb :77`, `DISPLAY=:77 GDK_BACKEND=x11
  GSK_RENDERER=cairo`) and screenshot with `import -display :77 -window root shot.png`.

## Logging

- Use `tracing` (`info!` for lifecycle, `debug!` for per-operation detail), never
  `println!`/`eprintln!` for diagnostics. CLI user output goes to stdout.
- Never log message bodies above `trace`; use `logging::body_summary()`.
- Log D-Bus failures with the error name (`error::dbus_error_name`) and enough context
  (device, folder, handle) to debug from a user's log file.

## Code notes

- zbus 6: `zbus::Variant`/`OwnedVariant` (not `Value`), proxies via `#[proxy]`, builder errors
  surface at `.build()`. sqlx 0.9: non-literal SQL needs `sqlx::AssertSqlSafe`.
- GTK callbacks run on the main thread; Bluetooth/DB work goes through `gui::runtime::run`.
  Never hold a `RefCell` borrow across `.await`.
- Phone numbers: `normalize_address` builds conversation/contact keys (region-dependent,
  never fails). Sending never uses those keys: it uses the number as typed
  (`normalize_for_sending` only strips formatting) or as the phone reported it
  (`db::reply_address`). A wrong region guess may split a thread but must never change who
  receives an SMS.
- Database changes: add a new migration at the end of `db::MIGRATIONS`; never edit applied ones.
  Users have real history in old databases.

## Dependencies

Add with `cargo add` (so the latest version is used), don't type versions by hand.

## Online help

```bash
ask-online 'How do I list files in Bash?'
```

## Documentation

Keep `README.md` (features, CLI, troubleshooting) and `docs/protocols.md` (protocol facts and
phone quirks) up to date with behaviour changes.
