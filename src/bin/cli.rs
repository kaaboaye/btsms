use anyhow::Result;
use btsms::bluetooth::{DeviceManager, MapClient, PbapClient};
use btsms::config::Config;
use btsms::contacts::ContactManager;
use btsms::db;
use btsms::sync::MessageSyncService;
use clap::{Parser, Subcommand};

/// BTSMS - Bluetooth SMS Manager CLI
#[derive(Parser)]
#[command(name = "btsms-cli")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Output format
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List paired Bluetooth devices
    Devices,

    /// Connect to a Bluetooth device
    Connect {
        /// Device Bluetooth address (e.g., AA:BB:CC:DD:EE:FF)
        address: String,
    },

    /// Disconnect from a Bluetooth device
    Disconnect {
        /// Device Bluetooth address (e.g., AA:BB:CC:DD:EE:FF)
        address: String,
    },

    /// Contact management commands
    #[command(subcommand)]
    Contacts(ContactsCommands),

    /// Message management commands
    #[command(subcommand)]
    Messages(MessagesCommands),
}

#[derive(Subcommand)]
enum ContactsCommands {
    /// Sync contacts from phone via PBAP
    Sync {
        /// Device Bluetooth address (auto-detects phone if not specified)
        #[arg(short, long)]
        address: Option<String>,
    },

    /// List contacts from local database
    List {
        /// Maximum number of contacts to display
        #[arg(short, long, default_value = "50")]
        limit: i64,
    },

    /// Search contacts by name or phone number
    Search {
        /// Search query
        query: String,
    },
}

#[derive(Subcommand)]
enum MessagesCommands {
    /// List recent messages from local database
    List {
        /// Maximum number of messages to display
        #[arg(short, long, default_value = "20")]
        limit: i64,
    },

    /// Fetch inbox messages from phone via MAP
    Inbox {
        /// Device Bluetooth address (auto-detects phone if not specified)
        #[arg(short, long)]
        address: Option<String>,
    },

    /// Fetch sent messages from phone via MAP
    Sent {
        /// Device Bluetooth address (auto-detects phone if not specified)
        #[arg(short, long)]
        address: Option<String>,
    },

    /// Send an SMS message via MAP
    Send {
        /// Recipient phone number
        recipient: String,

        /// Message text
        message: String,

        /// Device Bluetooth address (auto-detects phone if not specified)
        #[arg(short, long)]
        address: Option<String>,
    },

    /// Sync messages from phone to local database
    Sync {
        /// Device Bluetooth address (auto-detects phone if not specified)
        #[arg(short, long)]
        address: Option<String>,

        /// Only sync inbox messages
        #[arg(long)]
        inbox_only: bool,

        /// Only sync sent messages
        #[arg(long)]
        sent_only: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::default();

    // Initialize database
    let db_path = config.database_path.to_str().unwrap_or("btsms.db");
    let pool = db::init_database(db_path).await?;

    match cli.command {
        Commands::Devices => {
            cmd_devices(cli.json).await?;
        }
        Commands::Connect { address } => {
            cmd_connect(&address).await?;
        }
        Commands::Disconnect { address } => {
            cmd_disconnect(&address).await?;
        }
        Commands::Contacts(cmd) => match cmd {
            ContactsCommands::Sync { address } => {
                cmd_contacts_sync(address, &pool).await?;
            }
            ContactsCommands::List { limit } => {
                cmd_contacts_list(limit, &pool, cli.json).await?;
            }
            ContactsCommands::Search { query } => {
                cmd_contacts_search(&query, &pool, cli.json).await?;
            }
        },
        Commands::Messages(cmd) => match cmd {
            MessagesCommands::List { limit } => {
                cmd_messages_list(limit, &pool, cli.json).await?;
            }
            MessagesCommands::Inbox { address } => {
                cmd_messages_inbox(address).await?;
            }
            MessagesCommands::Sent { address } => {
                cmd_messages_sent(address).await?;
            }
            MessagesCommands::Send {
                recipient,
                message,
                address,
            } => {
                cmd_messages_send(&recipient, &message, address).await?;
            }
            MessagesCommands::Sync {
                address,
                inbox_only,
                sent_only,
            } => {
                cmd_messages_sync(address, inbox_only, sent_only, &pool, cli.json).await?;
            }
        },
    }

    Ok(())
}

/// List paired Bluetooth devices
async fn cmd_devices(json: bool) -> Result<()> {
    let device_manager = DeviceManager::new().await?;
    let devices = device_manager.get_paired_devices().await?;

    if json {
        let json_output: Vec<serde_json::Value> = devices
            .iter()
            .map(|d| {
                serde_json::json!({
                    "address": d.address,
                    "name": d.name,
                    "paired": d.paired,
                    "connected": d.connected,
                    "trusted": d.trusted,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_output)?);
    } else if devices.is_empty() {
        println!("No paired devices found.");
    } else {
        println!("{:<20} {:<30} {:<10} {:<10}", "ADDRESS", "NAME", "CONNECTED", "TRUSTED");
        println!("{}", "-".repeat(70));
        for device in devices {
            println!(
                "{:<20} {:<30} {:<10} {:<10}",
                device.address,
                truncate(&device.name, 28),
                if device.connected { "Yes" } else { "No" },
                if device.trusted { "Yes" } else { "No" }
            );
        }
    }

    Ok(())
}

/// Connect to a device
async fn cmd_connect(address: &str) -> Result<()> {
    let device_manager = DeviceManager::new().await?;
    device_manager.connect_device(address).await?;
    println!("Connected to {}", address);
    Ok(())
}

/// Disconnect from a device
async fn cmd_disconnect(address: &str) -> Result<()> {
    let device_manager = DeviceManager::new().await?;
    device_manager.disconnect_device(address).await?;
    println!("Disconnected from {}", address);
    Ok(())
}

/// Get device address, either from argument or auto-detect
async fn get_device_address(address: Option<String>) -> Result<String> {
    match address {
        Some(addr) => Ok(addr),
        None => {
            let device_manager = DeviceManager::new().await?;
            match device_manager.get_first_paired_phone().await? {
                Some(device) => {
                    eprintln!("Auto-detected phone: {} ({})", device.name, device.address);
                    Ok(device.address)
                }
                None => {
                    anyhow::bail!("No phone device found. Please specify --address")
                }
            }
        }
    }
}

/// Sync contacts from phone via PBAP
async fn cmd_contacts_sync(address: Option<String>, pool: &sqlx::SqlitePool) -> Result<()> {
    let device_address = get_device_address(address).await?;

    println!("Connecting to PBAP service...");
    let mut pbap_client = PbapClient::new(device_address.clone());
    pbap_client.connect().await?;

    println!("Pulling contacts...");
    let vcards = pbap_client.pull_all_contacts().await?;

    pbap_client.disconnect().await?;

    let contact_manager = ContactManager::new(pool.clone());
    let count = contact_manager.sync_from_vcards(&vcards, &device_address).await?;

    println!("Synced {} contacts from {}", count, device_address);

    Ok(())
}

/// List contacts from local database
async fn cmd_contacts_list(limit: i64, pool: &sqlx::SqlitePool, json: bool) -> Result<()> {
    let rows = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, display_name FROM contacts ORDER BY display_name LIMIT ?"
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    if json {
        let json_output: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, name)| {
                serde_json::json!({
                    "id": id,
                    "name": name,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_output)?);
    } else if rows.is_empty() {
        println!("No contacts found. Run 'btsms-cli contacts sync' to sync from phone.");
    } else {
        println!("{:<8} NAME", "ID");
        println!("{}", "-".repeat(50));
        for (id, name) in rows {
            println!("{:<8} {}", id, name);
        }
    }

    Ok(())
}

/// Search contacts by name or phone number
async fn cmd_contacts_search(query: &str, pool: &sqlx::SqlitePool, json: bool) -> Result<()> {
    let contact_manager = ContactManager::new(pool.clone());
    let contacts = contact_manager.search(query).await?;

    if json {
        let json_output: Vec<serde_json::Value> = contacts
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id,
                    "name": c.display_name,
                    "phone_numbers": c.phone_numbers.iter().map(|p| {
                        serde_json::json!({
                            "number": p.original,
                            "type": p.phone_type,
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_output)?);
    } else if contacts.is_empty() {
        println!("No contacts found matching '{}'", query);
    } else {
        for contact in contacts {
            println!("{} (ID: {})", contact.display_name, contact.id);
            for phone in &contact.phone_numbers {
                println!("  {} ({})", phone.original, phone.phone_type);
            }
        }
    }

    Ok(())
}

/// List recent messages from local database
async fn cmd_messages_list(limit: i64, pool: &sqlx::SqlitePool, json: bool) -> Result<()> {
    let messages = db::get_recent_messages(pool, limit).await?;

    if json {
        let json_output: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "sender": m.sender_number,
                    "sender_name": m.sender_name,
                    "recipient": m.recipient_number,
                    "body": m.body,
                    "timestamp": m.received_at,
                    "direction": format!("{}", m.direction),
                    "read": m.read_status,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_output)?);
    } else if messages.is_empty() {
        println!("No messages found.");
    } else {
        for msg in messages {
            let direction = match msg.direction {
                db::MessageDirection::Incoming => "<<",
                db::MessageDirection::Outgoing => ">>",
            };
            let from = msg.sender_name.as_ref().unwrap_or(&msg.sender_number);
            println!(
                "[{}] {} {} {}",
                &msg.received_at[..19],
                direction,
                from,
                if msg.read_status { "" } else { "(unread)" }
            );
            println!("    {}", truncate(&msg.body, 60));
            println!();
        }
    }

    Ok(())
}

/// Fetch inbox messages from phone via MAP
async fn cmd_messages_inbox(address: Option<String>) -> Result<()> {
    let device_address = get_device_address(address).await?;

    println!("Connecting to MAP service...");
    let mut map_client = MapClient::new(device_address);
    map_client.connect().await?;

    println!("Fetching inbox messages...");
    let messages = map_client.list_inbox_messages().await?;

    map_client.disconnect().await?;

    if messages.is_empty() {
        println!("No inbox messages found.");
    } else {
        println!("{:<20} {:<20} {:<40}", "TIMESTAMP", "FROM", "SUBJECT");
        println!("{}", "-".repeat(80));
        for msg in messages {
            println!(
                "{:<20} {:<20} {:<40}",
                truncate(&msg.timestamp, 18),
                truncate(&msg.sender, 18),
                truncate(&msg.subject, 38)
            );
        }
    }

    Ok(())
}

/// Fetch sent messages from phone via MAP
async fn cmd_messages_sent(address: Option<String>) -> Result<()> {
    let device_address = get_device_address(address).await?;

    println!("Connecting to MAP service...");
    let mut map_client = MapClient::new(device_address);
    map_client.connect().await?;

    println!("Fetching sent messages...");
    let messages = map_client.list_sent_messages().await?;

    map_client.disconnect().await?;

    if messages.is_empty() {
        println!("No sent messages found.");
    } else {
        println!("{:<20} {:<20} {:<40}", "TIMESTAMP", "TO", "SUBJECT");
        println!("{}", "-".repeat(80));
        for msg in messages {
            let recipient = msg.recipient.as_deref().unwrap_or("Unknown");
            println!(
                "{:<20} {:<20} {:<40}",
                truncate(&msg.timestamp, 18),
                truncate(recipient, 18),
                truncate(&msg.subject, 38)
            );
        }
    }

    Ok(())
}

/// Send an SMS message via MAP
async fn cmd_messages_send(recipient: &str, message: &str, address: Option<String>) -> Result<()> {
    let device_address = get_device_address(address).await?;

    println!("Connecting to MAP service...");
    let mut map_client = MapClient::new(device_address);
    map_client.connect().await?;

    println!("Sending message to {}...", recipient);
    map_client.send_sms(recipient, message).await?;

    map_client.disconnect().await?;

    println!("Message sent successfully!");

    Ok(())
}

/// Sync messages from phone to local database
async fn cmd_messages_sync(
    address: Option<String>,
    inbox_only: bool,
    sent_only: bool,
    pool: &sqlx::SqlitePool,
    json: bool,
) -> Result<()> {
    let device_address = get_device_address(address).await?;

    eprintln!("Connecting to MAP service...");
    let mut map_client = MapClient::new(device_address);
    map_client.connect().await?;

    eprintln!("Syncing messages...");

    let (inbox_imported, sent_imported, errors) = if inbox_only {
        let count = MessageSyncService::import_inbox(&map_client, pool)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Inbox sync error: {}", e);
                0
            });
        (count, 0, vec![])
    } else if sent_only {
        let count = MessageSyncService::import_sent(&map_client, pool).await;
        (0, count, vec![])
    } else {
        let result = MessageSyncService::sync_all(&map_client, pool).await;
        (result.inbox_imported, result.sent_imported, result.errors)
    };

    map_client.disconnect().await?;

    let total = inbox_imported + sent_imported;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "inbox_imported": inbox_imported,
                "sent_imported": sent_imported,
                "total": total,
                "errors": errors,
            }))?
        );
    } else {
        println!("Sync complete:");
        println!("  Inbox messages imported: {}", inbox_imported);
        println!("  Sent messages imported: {}", sent_imported);
        println!("  Total: {}", total);
        if !errors.is_empty() {
            println!("  Errors: {}", errors.len());
            for err in &errors {
                eprintln!("    - {}", err);
            }
        }
    }

    Ok(())
}

/// Truncate string to max characters with ellipsis (UTF-8 safe)
fn truncate(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_parsing() {
        // Verify CLI configuration is valid
        Cli::command().debug_assert();
    }

    #[test]
    fn test_devices_command_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "devices"]).unwrap();
        assert!(matches!(cli.command, Commands::Devices));
        assert!(!cli.json);
    }

    #[test]
    fn test_devices_command_with_json() {
        let cli = Cli::try_parse_from(["btsms-cli", "--json", "devices"]).unwrap();
        assert!(matches!(cli.command, Commands::Devices));
        assert!(cli.json);
    }

    #[test]
    fn test_connect_command_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "connect", "AA:BB:CC:DD:EE:FF"]).unwrap();
        if let Commands::Connect { address } = cli.command {
            assert_eq!(address, "AA:BB:CC:DD:EE:FF");
        } else {
            panic!("Expected Connect command");
        }
    }

    #[test]
    fn test_disconnect_command_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "disconnect", "AA:BB:CC:DD:EE:FF"]).unwrap();
        if let Commands::Disconnect { address } = cli.command {
            assert_eq!(address, "AA:BB:CC:DD:EE:FF");
        } else {
            panic!("Expected Disconnect command");
        }
    }

    #[test]
    fn test_contacts_sync_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "contacts", "sync"]).unwrap();
        if let Commands::Contacts(ContactsCommands::Sync { address }) = cli.command {
            assert!(address.is_none());
        } else {
            panic!("Expected Contacts Sync command");
        }
    }

    #[test]
    fn test_contacts_sync_with_address() {
        let cli = Cli::try_parse_from([
            "btsms-cli",
            "contacts",
            "sync",
            "--address",
            "AA:BB:CC:DD:EE:FF",
        ])
        .unwrap();
        if let Commands::Contacts(ContactsCommands::Sync { address }) = cli.command {
            assert_eq!(address, Some("AA:BB:CC:DD:EE:FF".to_string()));
        } else {
            panic!("Expected Contacts Sync command");
        }
    }

    #[test]
    fn test_contacts_list_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "contacts", "list"]).unwrap();
        if let Commands::Contacts(ContactsCommands::List { limit }) = cli.command {
            assert_eq!(limit, 50); // default value
        } else {
            panic!("Expected Contacts List command");
        }
    }

    #[test]
    fn test_contacts_list_with_limit() {
        let cli = Cli::try_parse_from(["btsms-cli", "contacts", "list", "--limit", "100"]).unwrap();
        if let Commands::Contacts(ContactsCommands::List { limit }) = cli.command {
            assert_eq!(limit, 100);
        } else {
            panic!("Expected Contacts List command");
        }
    }

    #[test]
    fn test_contacts_search_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "contacts", "search", "John"]).unwrap();
        if let Commands::Contacts(ContactsCommands::Search { query }) = cli.command {
            assert_eq!(query, "John");
        } else {
            panic!("Expected Contacts Search command");
        }
    }

    #[test]
    fn test_messages_list_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "messages", "list"]).unwrap();
        if let Commands::Messages(MessagesCommands::List { limit }) = cli.command {
            assert_eq!(limit, 20); // default value
        } else {
            panic!("Expected Messages List command");
        }
    }

    #[test]
    fn test_messages_list_with_limit() {
        let cli = Cli::try_parse_from(["btsms-cli", "messages", "list", "--limit", "50"]).unwrap();
        if let Commands::Messages(MessagesCommands::List { limit }) = cli.command {
            assert_eq!(limit, 50);
        } else {
            panic!("Expected Messages List command");
        }
    }

    #[test]
    fn test_messages_inbox_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "messages", "inbox"]).unwrap();
        if let Commands::Messages(MessagesCommands::Inbox { address }) = cli.command {
            assert!(address.is_none());
        } else {
            panic!("Expected Messages Inbox command");
        }
    }

    #[test]
    fn test_messages_sent_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "messages", "sent"]).unwrap();
        if let Commands::Messages(MessagesCommands::Sent { address }) = cli.command {
            assert!(address.is_none());
        } else {
            panic!("Expected Messages Sent command");
        }
    }

    #[test]
    fn test_messages_send_parsing() {
        let cli = Cli::try_parse_from([
            "btsms-cli",
            "messages",
            "send",
            "+15551234567",
            "Hello, World!",
        ])
        .unwrap();
        if let Commands::Messages(MessagesCommands::Send {
            recipient,
            message,
            address,
        }) = cli.command
        {
            assert_eq!(recipient, "+15551234567");
            assert_eq!(message, "Hello, World!");
            assert!(address.is_none());
        } else {
            panic!("Expected Messages Send command");
        }
    }

    #[test]
    fn test_messages_send_with_address() {
        let cli = Cli::try_parse_from([
            "btsms-cli",
            "messages",
            "send",
            "+15551234567",
            "Hello!",
            "--address",
            "AA:BB:CC:DD:EE:FF",
        ])
        .unwrap();
        if let Commands::Messages(MessagesCommands::Send {
            recipient,
            message,
            address,
        }) = cli.command
        {
            assert_eq!(recipient, "+15551234567");
            assert_eq!(message, "Hello!");
            assert_eq!(address, Some("AA:BB:CC:DD:EE:FF".to_string()));
        } else {
            panic!("Expected Messages Send command");
        }
    }

    #[test]
    fn test_messages_sync_parsing() {
        let cli = Cli::try_parse_from(["btsms-cli", "messages", "sync"]).unwrap();
        if let Commands::Messages(MessagesCommands::Sync {
            address,
            inbox_only,
            sent_only,
        }) = cli.command
        {
            assert!(address.is_none());
            assert!(!inbox_only);
            assert!(!sent_only);
        } else {
            panic!("Expected Messages Sync command");
        }
    }

    #[test]
    fn test_messages_sync_with_address() {
        let cli = Cli::try_parse_from([
            "btsms-cli",
            "messages",
            "sync",
            "--address",
            "AA:BB:CC:DD:EE:FF",
        ])
        .unwrap();
        if let Commands::Messages(MessagesCommands::Sync { address, .. }) = cli.command {
            assert_eq!(address, Some("AA:BB:CC:DD:EE:FF".to_string()));
        } else {
            panic!("Expected Messages Sync command");
        }
    }

    #[test]
    fn test_messages_sync_inbox_only() {
        let cli = Cli::try_parse_from(["btsms-cli", "messages", "sync", "--inbox-only"]).unwrap();
        if let Commands::Messages(MessagesCommands::Sync {
            inbox_only,
            sent_only,
            ..
        }) = cli.command
        {
            assert!(inbox_only);
            assert!(!sent_only);
        } else {
            panic!("Expected Messages Sync command");
        }
    }

    #[test]
    fn test_messages_sync_sent_only() {
        let cli = Cli::try_parse_from(["btsms-cli", "messages", "sync", "--sent-only"]).unwrap();
        if let Commands::Messages(MessagesCommands::Sync {
            inbox_only,
            sent_only,
            ..
        }) = cli.command
        {
            assert!(!inbox_only);
            assert!(sent_only);
        } else {
            panic!("Expected Messages Sync command");
        }
    }

    #[test]
    fn test_truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long_string() {
        assert_eq!(truncate("hello world!", 8), "hello...");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_utf8_multibyte() {
        // Test with Polish characters (multi-byte UTF-8)
        assert_eq!(truncate("żółć", 10), "żółć");
        // 10 chars = 7 content + 3 dots, "zażółć " is 7 chars
        assert_eq!(truncate("zażółć gęślą jaźń", 10), "zażółć ...");
        // Test the specific case that was crashing (no panic)
        let polish_text = "Ja jebe, to jest jakaś patologia że operatorzy wysyłają taki spam";
        let result = truncate(polish_text, 38);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= 38);
    }
}
