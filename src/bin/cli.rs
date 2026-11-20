use anyhow::{Context, Result};
use btsms::bluetooth::{AncsClient, DeviceManager, MapClient, MapFolder, MapMessage, vmessage};
use btsms::config::Config;
use btsms::contacts::{ContactManager, normalize_for_sending, phone_normalizer};
use btsms::db;
use btsms::service::{self, LinkEvent, LinkSettings, LinkState, PhoneLink};
use btsms::sync::{MessageSyncService, SyncResult, parse_map_time};
use clap::{Parser, Subcommand};
use sqlx::SqlitePool;
use std::path::PathBuf;

/// BTSMS - Bluetooth SMS Manager CLI
#[derive(Parser)]
#[command(name = "btsms-cli", author, version, about, long_about = None)]
struct Cli {
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    json: bool,

    /// More log output on stderr (-v info, -vv debug, -vvv trace)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Use this database instead of the configured one
    #[arg(long, global = true, value_name = "PATH")]
    database: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List paired Bluetooth devices
    Devices,

    /// Connect to a Bluetooth device
    Connect {
        /// Device address (e.g. AA:BB:CC:DD:EE:FF)
        address: String,
    },

    /// Disconnect from a Bluetooth device
    Disconnect {
        /// Device address (e.g. AA:BB:CC:DD:EE:FF)
        address: String,
    },

    /// Check BlueZ, obexd, MAP and ANCS step by step
    Doctor {
        /// Device address (auto-detects the phone if omitted)
        #[arg(short, long)]
        address: Option<String>,
    },

    /// Stay connected like the GUI does and print new messages as they arrive
    Listen {
        #[arg(short, long)]
        address: Option<String>,
    },

    /// Contact management
    #[command(subcommand)]
    Contacts(ContactsCommands),

    /// Messages
    #[command(subcommand)]
    Messages(MessagesCommands),

    /// List conversation threads from the local database
    Conversations {
        #[arg(short, long, default_value = "30")]
        limit: usize,
    },

    /// Print the log and data directories
    Paths,
}

#[derive(Subcommand)]
enum ContactsCommands {
    /// Download contacts from the phone (PBAP)
    Sync {
        #[arg(short, long)]
        address: Option<String>,
    },
    /// List contacts from the local database
    List {
        #[arg(short, long, default_value = "50")]
        limit: i64,
    },
    /// Search contacts by name or number
    Search { query: String },
}

#[derive(Subcommand)]
enum MessagesCommands {
    /// Recent messages from the local database
    List {
        #[arg(short, long, default_value = "20")]
        limit: i64,
        /// Only this conversation (number as stored, e.g. +48123456789)
        #[arg(long)]
        peer: Option<String>,
    },
    /// Show the phone's inbox (MAP), without storing anything
    Inbox {
        #[arg(short, long)]
        address: Option<String>,
        #[arg(short, long, default_value = "20")]
        limit: usize,
        /// Download and show the full text of each message
        #[arg(long)]
        full: bool,
    },
    /// Show the phone's sent folder (MAP); iPhones keep it empty
    Sent {
        #[arg(short, long)]
        address: Option<String>,
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Send an SMS
    Send {
        recipient: String,
        message: String,
        #[arg(short, long)]
        address: Option<String>,
        /// Show what would be sent without contacting the phone
        #[arg(long)]
        dry_run: bool,
    },
    /// Import messages from the phone into the local database
    Sync {
        #[arg(short, long)]
        address: Option<String>,
        #[arg(long, conflicts_with = "sent_only")]
        inbox_only: bool,
        #[arg(long)]
        sent_only: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let _log = btsms::logging::init("btsms-cli", "warn", cli.verbose);
    if let Err(e) = run(cli).await {
        tracing::error!(error = format!("{e:#}"), "command failed");
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let mut config = Config::load();
    if let Some(path) = &cli.database {
        config.database_path = path.clone();
    }
    phone_normalizer::set_default_region(config.region().as_deref());
    let json = cli.json;
    // Only the configured database absorbs the legacy messages.db, never an ad-hoc one.
    let merge_legacy = cli.database.is_none();

    match cli.command {
        Commands::Devices => cmd_devices(json).await,
        Commands::Connect { address } => {
            DeviceManager::new().await?.connect_device(&address).await?;
            println!("Connected to {address}");
            Ok(())
        }
        Commands::Disconnect { address } => {
            DeviceManager::new()
                .await?
                .disconnect_device(&address)
                .await?;
            println!("Disconnected from {address}");
            Ok(())
        }
        Commands::Doctor { address } => cmd_doctor(address, &config).await,
        Commands::Listen { address } => cmd_listen(address, &config, merge_legacy, json).await,
        Commands::Paths => {
            println!("config:   {}", Config::config_path().display());
            println!("database: {}", config.database_path.display());
            println!("logs:     {}", btsms::logging::log_dir().display());
            Ok(())
        }
        Commands::Conversations { limit } => {
            let pool = open_db(&config, merge_legacy).await?;
            let convs = db::get_conversations(&pool).await?;
            let convs = &convs[..convs.len().min(limit)];
            if json {
                print_json(&convs)?;
            } else if convs.is_empty() {
                println!("No conversations yet. Run 'btsms-cli messages sync'.");
            } else {
                for c in convs {
                    let unread = if c.unread_count > 0 {
                        format!(" ({} unread)", c.unread_count)
                    } else {
                        String::new()
                    };
                    println!(
                        "{}  {}{}\n    {}{}",
                        local_time(&c.last_message_time),
                        c.title(),
                        unread,
                        if c.is_outgoing { "You: " } else { "" },
                        truncate(&c.last_message.replace('\n', " "), 70)
                    );
                }
            }
            Ok(())
        }
        Commands::Contacts(cmd) => {
            let pool = open_db(&config, merge_legacy).await?;
            match cmd {
                ContactsCommands::Sync { address } => {
                    let address = resolve_address(address).await?;
                    eprintln!("Downloading contacts from {address}...");
                    let count = service::sync_contacts(&address, &pool).await?;
                    println!("Synced {count} contacts from {address}");
                    Ok(())
                }
                ContactsCommands::List { limit } => {
                    let contacts = ContactManager::new(pool).list(limit).await?;
                    if json {
                        let rows: Vec<_> = contacts
                            .iter()
                            .map(|(id, name)| serde_json::json!({"id": id, "name": name}))
                            .collect();
                        print_json(&rows)?;
                    } else if contacts.is_empty() {
                        println!("No contacts. Run 'btsms-cli contacts sync'.");
                    } else {
                        for (id, name) in contacts {
                            println!("{id:<6} {name}");
                        }
                    }
                    Ok(())
                }
                ContactsCommands::Search { query } => {
                    let contacts = ContactManager::new(pool).search(&query).await?;
                    if json {
                        print_json(&contacts)?;
                    } else if contacts.is_empty() {
                        println!("No contacts matching '{query}'");
                    } else {
                        for c in contacts {
                            println!("{} (#{})", c.display_name, c.id);
                            for p in &c.phone_numbers {
                                println!("  {} [{}] -> {}", p.original, p.phone_type, p.normalized);
                            }
                        }
                    }
                    Ok(())
                }
            }
        }
        Commands::Messages(cmd) => match cmd {
            MessagesCommands::List { limit, peer } => {
                let pool = open_db(&config, merge_legacy).await?;
                let messages = match peer {
                    Some(peer) => db::get_messages_for_conversation(&pool, &peer, limit).await?,
                    None => db::get_recent_messages(&pool, limit).await?,
                };
                if json {
                    print_json(&messages)?;
                } else if messages.is_empty() {
                    println!("No messages.");
                } else {
                    for m in messages {
                        let arrow = match m.direction {
                            db::MessageDirection::Incoming => "<<",
                            db::MessageDirection::Outgoing => ">>",
                        };
                        let who = m.sender_name.as_deref().unwrap_or(&m.peer);
                        let unread = if m.read_status { "" } else { " (unread)" };
                        println!("[{}] {arrow} {who}{unread}", local_time(&m.received_at));
                        println!("    {}\n", truncate(&m.body.replace('\n', " "), 70));
                    }
                }
                Ok(())
            }
            MessagesCommands::Inbox {
                address,
                limit,
                full,
            } => cmd_show_folder(address, MapFolder::Inbox, limit, full, json).await,
            MessagesCommands::Sent { address, limit } => {
                cmd_show_folder(address, MapFolder::Sent, limit, false, json).await
            }
            MessagesCommands::Send {
                recipient,
                message,
                address,
                dry_run,
            } => {
                let number = normalize_for_sending(&recipient)?;
                if dry_run {
                    println!("Recipient: {recipient} -> {number}");
                    println!(
                        "{}",
                        vmessage::create_bmessage(&number, &message).replace("\r\n", "\n")
                    );
                    return Ok(());
                }
                let pool = open_db(&config, merge_legacy).await?;
                let address = resolve_address(address).await?;
                let mut map = connect_map(&address).await?;
                eprintln!("Sending to {number}...");
                let result = service::send_and_store(&map, &pool, &number, &message).await;
                map.disconnect().await;
                let stored = result?;
                if json {
                    print_json(&stored)?;
                } else {
                    println!("Message sent to {number}");
                }
                Ok(())
            }
            MessagesCommands::Sync {
                address,
                inbox_only,
                sent_only,
            } => {
                let pool = open_db(&config, merge_legacy).await?;
                let address = resolve_address(address).await?;
                let mut map = connect_map(&address).await?;
                eprintln!("Syncing messages...");
                let mut result = SyncResult::default();
                if !sent_only {
                    MessageSyncService::sync_folder(&map, &pool, MapFolder::Inbox, &mut result)
                        .await;
                }
                if !inbox_only {
                    MessageSyncService::sync_folder(&map, &pool, MapFolder::Sent, &mut result)
                        .await;
                }
                map.disconnect().await;
                if json {
                    print_json(&result)?;
                } else {
                    println!("Sync complete:");
                    println!("  Inbox messages imported: {}", result.inbox_imported);
                    println!("  Sent messages imported:  {}", result.sent_imported);
                    println!("  Already stored:          {}", result.already_known);
                    if result.download_failures > 0 {
                        println!(
                            "  Stored with preview only: {} (full text download failed)",
                            result.download_failures
                        );
                    }
                    for err in &result.errors {
                        println!("  Error: {err}");
                    }
                }
                if result.errors.is_empty() {
                    Ok(())
                } else {
                    anyhow::bail!("{} error(s) during sync", result.errors.len())
                }
            }
        },
    }
}

async fn open_db(config: &Config, merge_legacy: bool) -> Result<SqlitePool> {
    db::open_app_database(config, merge_legacy)
        .await
        .with_context(|| format!("cannot open database {}", config.database_path.display()))
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

async fn resolve_address(address: Option<String>) -> Result<String> {
    if let Some(addr) = address {
        return Ok(addr);
    }
    let manager = DeviceManager::new().await?;
    match manager.get_first_paired_phone().await? {
        Some(device) => {
            eprintln!("Using phone: {} ({})", device.name, device.address);
            Ok(device.address)
        }
        None => anyhow::bail!("No paired phone found. Pair one first or pass --address"),
    }
}

async fn connect_map(address: &str) -> Result<MapClient> {
    eprintln!("Connecting to MAP on {address}...");
    let mut map = MapClient::new(address.to_string());
    map.connect().await?;
    Ok(map)
}

async fn cmd_devices(json: bool) -> Result<()> {
    let devices = DeviceManager::new().await?.get_paired_devices().await?;
    if json {
        let rows: Vec<_> = devices
            .iter()
            .map(|d| {
                serde_json::json!({
                    "address": d.address,
                    "name": d.name,
                    "icon": d.icon,
                    "is_phone": d.is_phone(),
                    "paired": d.paired,
                    "connected": d.connected,
                    "trusted": d.trusted,
                })
            })
            .collect();
        return print_json(&rows);
    }
    if devices.is_empty() {
        println!("No paired devices found.");
        return Ok(());
    }
    println!(
        "{:<19} {:<28} {:<6} {:<10} TRUSTED",
        "ADDRESS", "NAME", "PHONE", "CONNECTED"
    );
    for d in devices {
        println!(
            "{:<19} {:<28} {:<6} {:<10} {}",
            d.address,
            truncate(&d.name, 27),
            yes_no(d.is_phone()),
            yes_no(d.connected),
            yes_no(d.trusted)
        );
    }
    Ok(())
}

async fn cmd_show_folder(
    address: Option<String>,
    folder: MapFolder,
    limit: usize,
    full: bool,
    json: bool,
) -> Result<()> {
    let address = resolve_address(address).await?;
    let mut map = connect_map(&address).await?;
    let result = async {
        let mut messages = map.list_messages(folder).await?;
        messages.reverse(); // newest first
        messages.truncate(limit);
        let mut rows = Vec::new();
        for m in &messages {
            let text = if full && !m.subject_is_complete() {
                match map.get_message(m).await {
                    Ok(b) => b.body,
                    Err(e) => format!("<download failed: {e}> {}", m.subject),
                }
            } else {
                m.subject.clone()
            };
            rows.push((m.clone(), text));
        }
        Ok::<_, anyhow::Error>(rows)
    }
    .await;
    map.disconnect().await;
    let rows = result?;

    if json {
        let out: Vec<_> = rows
            .iter()
            .map(|(m, text)| {
                serde_json::json!({
                    "handle": m.handle,
                    "timestamp": m.timestamp,
                    "sender": m.sender,
                    "sender_address": m.sender_address,
                    "recipient": m.recipient,
                    "recipient_address": m.recipient_address,
                    "type": m.message_type,
                    "size": m.size,
                    "read": m.read,
                    "text": text,
                })
            })
            .collect();
        return print_json(&out);
    }
    if rows.is_empty() {
        println!("No messages in {folder}.");
        return Ok(());
    }
    println!(
        "{:<17} {:<22} TEXT",
        "TIME",
        if folder == MapFolder::Inbox {
            "FROM"
        } else {
            "TO"
        }
    );
    for (m, text) in rows {
        println!(
            "{:<17} {:<22} {}{}",
            map_time(&m),
            truncate(&party(&m, folder), 21),
            if m.read { "" } else { "* " },
            truncate(&text.replace('\n', " "), 60)
        );
    }
    Ok(())
}

fn party(m: &MapMessage, folder: MapFolder) -> String {
    if folder == MapFolder::Inbox {
        m.sender_address.clone().unwrap_or_else(|| m.sender.clone())
    } else {
        m.recipient_address
            .clone()
            .or_else(|| m.recipient.clone())
            .unwrap_or_else(|| "?".into())
    }
}

fn map_time(m: &MapMessage) -> String {
    parse_map_time(&m.timestamp)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| m.timestamp.clone())
}

/// Step-by-step diagnosis of the Bluetooth stack, for bug reports.
async fn cmd_doctor(address: Option<String>, config: &Config) -> Result<()> {
    let ok = |s: &str| println!("  [ok]   {s}");
    let bad = |s: &str| println!("  [FAIL] {s}");
    let info = |s: &str| println!("  [info] {s}");

    println!("btsms {}", env!("CARGO_PKG_VERSION"));
    info(&format!("logs: {}", btsms::logging::log_dir().display()));
    info(&format!("database: {}", config.database_path.display()));
    info(&format!(
        "phone number region: {}",
        config
            .region()
            .unwrap_or_else(|| "none (numbers without +CC are kept as typed)".into())
    ));

    println!("BlueZ:");
    let manager = match DeviceManager::new().await {
        Ok(m) => m,
        Err(e) => {
            bad(&format!("system D-Bus: {e}"));
            return Ok(());
        }
    };
    let phones = match manager.get_all_paired_phones().await {
        Ok(p) => {
            ok(&format!(
                "bluetoothd reachable, {} paired phone(s)",
                p.len()
            ));
            p
        }
        Err(e) => {
            bad(&format!("{e}"));
            return Ok(());
        }
    };
    let address = match address.or_else(|| {
        phones
            .iter()
            .find(|p| p.connected)
            .or(phones.first())
            .map(|p| p.address.clone())
    }) {
        Some(a) => a,
        None => {
            bad("no paired phone; pair it in your system Bluetooth settings first");
            return Ok(());
        }
    };
    match manager.find_device(&address).await {
        Ok(d) => {
            info(&format!(
                "phone: {} ({}) icon={:?}",
                d.name, d.address, d.icon
            ));
            if d.connected {
                ok("phone is connected");
            } else {
                bad("phone is not connected; trying to connect");
                match manager.connect_device(&address).await {
                    Ok(()) => ok("connected"),
                    Err(e) => bad(&format!("{e}")),
                }
            }
        }
        Err(e) => bad(&format!("{e}")),
    }

    println!("MAP (messages):");
    let mut map = MapClient::new(address.clone());
    match map.connect().await {
        Ok(()) => {
            ok("session open");
            for folder in [MapFolder::Inbox, MapFolder::Sent] {
                match map.list_messages(folder).await {
                    Ok(list) => {
                        ok(&format!("{folder}: {} message(s)", list.len()));
                        if folder == MapFolder::Inbox
                            && let Some(m) = list.iter().rev().find(|m| !m.subject.is_empty())
                        {
                            match map.get_message(m).await {
                                Ok(b) => ok(&format!(
                                    "full text download works ({} chars, sender vCard: {})",
                                    b.body.chars().count(),
                                    if b.originator_phone().is_some() {
                                        "yes"
                                    } else {
                                        "no"
                                    }
                                )),
                                Err(e) => bad(&format!("full text download: {e}")),
                            }
                        }
                    }
                    Err(e) => bad(&format!("{folder}: {e}")),
                }
            }
            map.disconnect().await;
        }
        Err(e) => bad(&format!("{e}")),
    }

    println!("ANCS (iPhone live notifications):");
    match AncsClient::connect(&address).await {
        Ok(_) => ok("ANCS service found"),
        Err(e) => info(&format!("not available: {e}")),
    }
    Ok(())
}

async fn cmd_listen(
    address: Option<String>,
    config: &Config,
    merge_legacy: bool,
    json: bool,
) -> Result<()> {
    let pool = open_db(config, merge_legacy).await?;
    let address = resolve_address(address).await?;
    let device = DeviceManager::new().await?.find_device(&address).await?;
    let (link, mut events) = PhoneLink::start(device, pool, LinkSettings::from_config(config));
    eprintln!("Listening on {address}; Ctrl+C to stop.");
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = events.recv() => {
                let Some(event) = event else { break };
                match event {
                    LinkEvent::State(state) => {
                        if !json {
                            eprintln!("state: {}", describe_state(&state));
                        }
                        if state == LinkState::Stopped {
                            break;
                        }
                    }
                    LinkEvent::NewMessages { messages, initial } => {
                        for m in messages {
                            if json {
                                println!("{}", serde_json::to_string(&serde_json::json!({
                                    "initial": initial, "message": m
                                }))?);
                            } else {
                                let who = m.sender_name.as_deref().unwrap_or(&m.peer);
                                let arrow = match m.direction {
                                    db::MessageDirection::Incoming => "<<",
                                    db::MessageDirection::Outgoing => ">>",
                                };
                                println!("[{}] {arrow} {who}: {}", local_time(&m.received_at), m.body);
                            }
                        }
                    }
                    LinkEvent::SyncFailed(e) => eprintln!("sync error: {e}"),
                }
            }
        }
    }
    link.stop().await;
    Ok(())
}

fn describe_state(state: &LinkState) -> String {
    match state {
        LinkState::Connecting => "connecting".into(),
        LinkState::Connected { live_notifications } => format!(
            "connected ({})",
            if *live_notifications {
                "live notifications"
            } else {
                "polling"
            }
        ),
        LinkState::Reconnecting {
            attempt,
            retry_in,
            reason,
        } => format!(
            "reconnecting in {}s (attempt {attempt}): {reason}",
            retry_in.as_secs()
        ),
        LinkState::Stopped => "stopped".into(),
    }
}

fn local_time(rfc3339: &str) -> String {
    db::parse_db_time(rfc3339)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| rfc3339.to_string())
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// Truncate to `max_chars` characters with an ellipsis (UTF-8 safe).
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_work_after_subcommand() {
        let cli = Cli::try_parse_from(["btsms-cli", "devices", "--json", "-vv"]).unwrap();
        assert!(cli.json);
        assert_eq!(cli.verbose, 2);
    }

    #[test]
    fn send_parsing() {
        let cli = Cli::try_parse_from([
            "btsms-cli",
            "messages",
            "send",
            "+48512345678",
            "Hello!",
            "-a",
            "AA:BB",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Commands::Messages(MessagesCommands::Send {
                recipient,
                message,
                address,
                dry_run,
            }) => {
                assert_eq!(recipient, "+48512345678");
                assert_eq!(message, "Hello!");
                assert_eq!(address.as_deref(), Some("AA:BB"));
                assert!(dry_run);
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn sync_flags_conflict() {
        assert!(
            Cli::try_parse_from([
                "btsms-cli",
                "messages",
                "sync",
                "--inbox-only",
                "--sent-only"
            ])
            .is_err()
        );
    }

    #[test]
    fn database_override() {
        let cli = Cli::try_parse_from(["btsms-cli", "--database", "/tmp/x.db", "paths"]).unwrap();
        assert_eq!(cli.database, Some(PathBuf::from("/tmp/x.db")));
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("exactly10!", 10), "exactly10!");
        assert_eq!(truncate("Zażółć gęślą jaźń", 6), "Zażół…");
        assert_eq!(truncate("👋👋👋", 2), "👋…");
    }

    #[test]
    fn local_time_falls_back_to_input() {
        assert_eq!(local_time("not a time"), "not a time");
        assert!(local_time("2026-01-01T12:00:00Z").starts_with("2026-01-01"));
    }
}
