use crate::config::{AppConfig, ConfigStore, default_profile_root, default_state_dir};
use crate::daemon;
use crate::errors::IrisError;
use crate::ir::{DryRunTransmitter, IrSignal, IrTransmitter, RppalTransmitter, describe_signal};
use crate::profiles::{ProfileId, ProfileStore};
use crate::server;
use clap::{Args, Parser, Subcommand};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "iris",
    version,
    about = "Control infrared TVs from a Raspberry Pi"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Start(DeviceArgs),
    Send(SendArgs),
    Scan(ScanArgs),
    List {
        #[command(subcommand)]
        command: ListCommands,
    },
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },
    HomeAssistant {
        #[command(subcommand)]
        command: HomeAssistantCommands,
    },
    Serve(DeviceArgs),
    Status,
}

#[derive(Debug, Args)]
pub struct SendArgs {
    pub command: String,
    #[arg(long)]
    pub repeat: Option<u32>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    #[arg(default_value = "power")]
    pub command: String,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub repeat: Option<u32>,
    #[arg(long, help = "Maximum number of scan candidates to send")]
    pub limit: Option<usize>,
    #[arg(long, default_value_t = 1000, help = "Delay between bomb candidates")]
    pub interval_ms: u64,
    #[arg(long, help = "Do not wait for Enter between candidates")]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct DeviceArgs {
    pub brand: String,
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ListCommands {
    Brands,
    Models { brand: String },
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommands {
    Show { profile: String },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    Set { key: String, value: String },
}

#[derive(Debug, Subcommand)]
pub enum DaemonCommands {
    Start(DeviceArgs),
    Stop,
}

#[derive(Debug, Subcommand)]
pub enum HomeAssistantCommands {
    Setup,
}

pub async fn run(cli: Cli) -> Result<(), IrisError> {
    let config_store = ConfigStore::from_environment()?;
    let profile_store = ProfileStore::new(default_profile_root());

    match cli.command {
        Commands::Start(args) => start_profile(&config_store, &profile_store, &args),
        Commands::Send(args) => send_command(&config_store, &profile_store, args),
        Commands::Scan(args) => scan_command(&config_store, args),
        Commands::List { command } => list_command(&profile_store, command),
        Commands::Profile { command } => profile_command(&profile_store, command),
        Commands::Config { command } => config_command(&config_store, command),
        Commands::Daemon { command } => daemon_command(&config_store, &profile_store, command),
        Commands::HomeAssistant { command } => home_assistant_command(&config_store, command),
        Commands::Serve(args) => serve(&config_store, &profile_store, &args).await,
        Commands::Status => status(&config_store),
    }
}

#[derive(Debug, Clone)]
struct ScanCandidate {
    name: String,
    signal: IrSignal,
    frequency: u32,
}

fn scan_command(config_store: &ConfigStore, args: ScanArgs) -> Result<(), IrisError> {
    let config = config_store.load()?;
    let repeat = args.repeat.unwrap_or(config.default_repeat).max(1);

    if args.command == "bomb" {
        return scan_bomb(&config, &args, repeat);
    }

    if args.command != "power" {
        return Err(IrisError::CommandNotFound {
            command: args.command,
            profile: "scan".to_string(),
        });
    }

    let candidates = power_scan_candidates();
    println!("Scanning power candidates ({})", candidates.len());
    println!("Point the IR LED at the TV. Press Enter to send each candidate.");
    println!("repeat = {repeat}");

    for (idx, candidate) in candidates.iter().enumerate() {
        if !args.yes {
            println!(
                "[{}/{}] Press Enter to try {}",
                idx + 1,
                candidates.len(),
                candidate.name
            );
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
        }

        println!(
            "[{}/{}] {}: {}",
            idx + 1,
            candidates.len(),
            candidate.name,
            describe_signal(&candidate.signal, repeat)
        );

        if args.dry_run {
            continue;
        }

        let tx = RppalTransmitter::new(config.gpio_pin, candidate.frequency)?;
        tx.send(candidate.signal.clone(), repeat)?;
    }

    println!("Scan complete. If one worked, note its candidate name.");
    Ok(())
}

fn scan_bomb(config: &AppConfig, args: &ScanArgs, repeat: u32) -> Result<(), IrisError> {
    let candidates = bomb_scan_candidates();
    let limit = args.limit.unwrap_or(candidates.len()).min(candidates.len());
    println!("Bombing IR candidates ({limit}/{})", candidates.len());
    println!(
        "Point the IR LED at the TV. Sending one candidate every {} ms.",
        args.interval_ms
    );
    println!("repeat = {repeat}");

    for (idx, candidate) in candidates.iter().take(limit).enumerate() {
        println!(
            "[{}/{}] {}: {}",
            idx + 1,
            limit,
            candidate.name,
            describe_signal(&candidate.signal, repeat)
        );

        if !args.dry_run {
            let tx = RppalTransmitter::new(config.gpio_pin, candidate.frequency)?;
            tx.send(candidate.signal.clone(), repeat)?;
        }

        if idx + 1 < limit && args.interval_ms > 0 {
            std::thread::sleep(Duration::from_millis(args.interval_ms));
        }
    }

    println!("Bomb scan complete. If the TV reacted, note the last candidate name.");
    Ok(())
}

fn power_scan_candidates() -> Vec<ScanCandidate> {
    vec![
        ScanCandidate {
            name: "tcl_nikai_power".to_string(),
            signal: IrSignal::Nikai {
                data: 0x0D5F2A,
                bits: 24,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_36khz".to_string(),
            signal: IrSignal::Nikai {
                data: 0x0D5F2A,
                bits: 24,
            },
            frequency: 36_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_40khz".to_string(),
            signal: IrSignal::Nikai {
                data: 0x0D5F2A,
                bits: 24,
            },
            frequency: 40_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_1".to_string(),
            signal: IrSignal::Nikai {
                data: 0x0CF30C,
                bits: 24,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_1_36khz".to_string(),
            signal: IrSignal::Nikai {
                data: 0x0CF30C,
                bits: 24,
            },
            frequency: 36_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_1_40khz".to_string(),
            signal: IrSignal::Nikai {
                data: 0x0CF30C,
                bits: 24,
            },
            frequency: 40_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_2".to_string(),
            signal: IrSignal::Nikai {
                data: 0x0CFF30,
                bits: 24,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_3".to_string(),
            signal: IrSignal::Nikai {
                data: 0x0D0F2F,
                bits: 24,
            },
            frequency: 38_000,
        },
        nec_candidate("nec_00ff_a25d", 0x00FF, 0xA25D),
        nec_candidate("nec_00ff_45ba", 0x00FF, 0x45BA),
        nec_candidate("nec_00ff_e21d", 0x00FF, 0xE21D),
        nec_candidate("nec_807f_02fd", 0x807F, 0x02FD),
        nec_candidate("nec_807f_12ed", 0x807F, 0x12ED),
        nec_candidate("nec_807f_48b7", 0x807F, 0x48B7),
        nec_candidate("nec_04fb_08f7", 0x04FB, 0x08F7),
        nec_candidate("nec_04fb_0cf3", 0x04FB, 0x0CF3),
        nec_candidate("nec_e0e0_40bf", 0xE0E0, 0x40BF),
        nec_candidate("nec_e0e0_f20d", 0xE0E0, 0xF20D),
        nec_candidate("nec_20df_10ef", 0x20DF, 0x10EF),
        nec_candidate("nec_20df_23dc", 0x20DF, 0x23DC),
        nec_candidate("nec_bf40_12ed", 0xBF40, 0x12ED),
        nec_candidate("nec_7f80_02fd", 0x7F80, 0x02FD),
        nec_candidate("nec_f708_fb04", 0xF708, 0xFB04),
        nec_candidate("nec_f20d_e01f", 0xF20D, 0xE01F),
        nec_candidate("nec_10ef_0ff0", 0x10EF, 0x0FF0),
        nec_candidate("nec_1fe0_08f7", 0x1FE0, 0x08F7),
        nec_candidate("nec_df20_10ef", 0xDF20, 0x10EF),
        nec_candidate("nec_ff00_a25d", 0xFF00, 0xA25D),
        nec_candidate("nec_0af5_10ef", 0x0AF5, 0x10EF),
        nec_candidate("nec_40bf_12ed", 0x40BF, 0x12ED),
    ]
}

fn bomb_scan_candidates() -> Vec<ScanCandidate> {
    let mut candidates = Vec::new();
    let nikai_commands = [
        ("power", 0x0D5F2A),
        ("volume_up", 0x0D0F2F),
        ("volume_down", 0x0D1F2E),
        ("mute", 0x0C0F3F),
        ("input", 0x05CFA3),
        ("channel_up", 0x0D2F2D),
        ("channel_down", 0x0D3F2C),
        ("menu", 0x013FEC),
        ("q_menu", 0x030FCF),
        ("up", 0x0A6F59),
        ("down", 0x0A7F58),
        ("left", 0x0A9F56),
        ("right", 0x0A8F57),
        ("ok", 0x00BFF4),
        ("back", 0x0D8F27),
        ("home", 0x0F7F08),
        ("netflix", 0x010FEF),
        ("prime_video", 0x03EFC1),
        ("youtube", 0x01DFE2),
        ("red", 0x0FFF00),
        ("green", 0x017FE8),
        ("yellow", 0x01BFE4),
        ("blue", 0x027FD8),
        ("info", 0x0C3F3C),
        ("list", 0x09EF61),
    ];
    for (command, data) in nikai_commands {
        candidates.push(ScanCandidate {
            name: format!("tcl_nikai_{command}"),
            signal: IrSignal::Nikai { data, bits: 24 },
            frequency: 38_000,
        });
    }
    for frequency in [36_000, 40_000] {
        for (command, data) in nikai_commands {
            candidates.push(ScanCandidate {
                name: format!("tcl_nikai_{command}_{frequency}hz"),
                signal: IrSignal::Nikai { data, bits: 24 },
                frequency,
            });
        }
    }

    let nec_addresses = [0x00FF, 0x807F, 0x04FB, 0xE0E0, 0x20DF, 0xBF40];
    let nec_commands = [
        0xA25D, 0x45BA, 0xE21D, 0x02FD, 0x12ED, 0x48B7, 0x08F7, 0x0CF3, 0x40BF, 0xF20D, 0x10EF,
        0x23DC, 0xE01F, 0x0FF0, 0x22DD, 0x629D, 0xA857, 0x18E7, 0x4AB5, 0x9867,
    ];
    for address in nec_addresses {
        for command in nec_commands {
            candidates.push(nec_candidate(
                &format!("nec_{address:04x}_{command:04x}"),
                address,
                command,
            ));
        }
    }

    candidates
}

fn nec_candidate(name: &str, address: u16, command: u16) -> ScanCandidate {
    ScanCandidate {
        name: name.to_string(),
        signal: IrSignal::Nec { address, command },
        frequency: 38_000,
    }
}

fn start_profile(
    config_store: &ConfigStore,
    profile_store: &ProfileStore,
    args: &DeviceArgs,
) -> Result<(), IrisError> {
    let loaded = profile_store.load_brand_model(&args.brand, args.model.as_deref())?;
    let mut config = config_store.load()?;
    config.active_profile = Some(loaded.id());
    config_store.save(&config)?;
    println!("Loaded active profile {}", loaded.id());
    Ok(())
}

fn send_command(
    config_store: &ConfigStore,
    profile_store: &ProfileStore,
    args: SendArgs,
) -> Result<(), IrisError> {
    let config = config_store.load()?;
    let active_profile = config
        .active_profile
        .as_deref()
        .ok_or(IrisError::ActiveProfileMissing)?;
    let profile = profile_store.load(active_profile)?;
    let signal = profile.signal_for(&args.command)?;
    let repeat = args.repeat.unwrap_or(config.default_repeat).max(1);

    if args.dry_run {
        let tx = DryRunTransmitter::new();
        tx.send(signal, repeat)?;
        return Ok(());
    }

    let tx = RppalTransmitter::new(config.gpio_pin, effective_frequency(&config, &profile))?;
    tx.send(signal, repeat)
}

fn list_command(profile_store: &ProfileStore, command: ListCommands) -> Result<(), IrisError> {
    match command {
        ListCommands::Brands => {
            for brand in profile_store.list_brands()? {
                println!("{brand}");
            }
        }
        ListCommands::Models { brand } => {
            for model in profile_store.list_models(&brand)? {
                println!("{model}");
            }
        }
    }
    Ok(())
}

fn profile_command(
    profile_store: &ProfileStore,
    command: ProfileCommands,
) -> Result<(), IrisError> {
    match command {
        ProfileCommands::Show { profile } => {
            let loaded = profile_store.load(&profile)?;
            println!("brand = {}", loaded.brand);
            println!("model = {}", loaded.model);
            println!("device_type = {}", loaded.device_type);
            println!("commands = {}", loaded.commands.len());
            for command in loaded.commands.keys() {
                println!("- {command}");
            }
        }
    }
    Ok(())
}

fn config_command(config_store: &ConfigStore, command: ConfigCommands) -> Result<(), IrisError> {
    match command {
        ConfigCommands::Set { key, value } => {
            let _ = config_store.set(&key, &value)?;
            println!("Set {key} = {value}");
        }
    }
    Ok(())
}

fn home_assistant_command(
    config_store: &ConfigStore,
    command: HomeAssistantCommands,
) -> Result<(), IrisError> {
    match command {
        HomeAssistantCommands::Setup => {
            let config = config_store.prepare_home_assistant()?;
            println!("Home Assistant discovery is ready");
            println!("device_id = {}", config.device_id.as_deref().unwrap_or(""));
            println!("device_name = {}", config.device_name);
            println!("server = {}:{}", config.server_host, config.server_port);
            println!("api_token = {}", config.api_token.as_deref().unwrap_or(""));
            println!(
                "Install the IRIS custom integration in Home Assistant, then accept the discovered device."
            );
        }
    }
    Ok(())
}

fn daemon_command(
    config_store: &ConfigStore,
    profile_store: &ProfileStore,
    command: DaemonCommands,
) -> Result<(), IrisError> {
    let state_dir = default_state_dir()?;
    match command {
        DaemonCommands::Start(args) => {
            let loaded = profile_store.load_brand_model(&args.brand, args.model.as_deref())?;
            let mut config = config_store.load()?;
            config.active_profile = Some(loaded.id());
            config_store.save(&config)?;
            let pid = daemon::start(&state_dir, &loaded.id())?;
            println!("Started IRIS daemon for {} with PID {pid}", loaded.id());
        }
        DaemonCommands::Stop => {
            daemon::stop(&state_dir)?;
            println!("Stopped IRIS daemon");
        }
    }
    Ok(())
}

async fn serve(
    config_store: &ConfigStore,
    profile_store: &ProfileStore,
    args: &DeviceArgs,
) -> Result<(), IrisError> {
    let loaded = load_device_or_profile(profile_store, args)?;
    let mut config = config_store.load()?;
    config.active_profile = Some(loaded.id());
    config_store.save(&config)?;
    let frequency = effective_frequency(&config, &loaded);
    let transmitter = Arc::new(RppalTransmitter::new(config.gpio_pin, frequency)?);
    server::serve(loaded, config, transmitter).await
}

fn load_device_or_profile(
    profile_store: &ProfileStore,
    args: &DeviceArgs,
) -> Result<crate::profiles::Profile, IrisError> {
    if args.model.is_none() && args.brand.contains('/') {
        profile_store.load(&args.brand)
    } else {
        profile_store.load_brand_model(&args.brand, args.model.as_deref())
    }
}

fn status(config_store: &ConfigStore) -> Result<(), IrisError> {
    let config = config_store.load()?;
    println!(
        "active_profile = {}",
        config.active_profile.as_deref().unwrap_or("<none>")
    );
    println!("gpio_pin = {}", config.gpio_pin);
    println!("carrier_frequency = {}", config.carrier_frequency);
    println!("server = {}:{}", config.server_host, config.server_port);
    println!(
        "device_id = {}",
        config.device_id.as_deref().unwrap_or("<none>")
    );
    println!("device_name = {}", config.device_name);
    println!("discovery_enabled = {}", config.discovery_enabled);
    Ok(())
}

fn effective_frequency(config: &AppConfig, profile: &crate::profiles::Profile) -> u32 {
    profile
        .carrier_frequency
        .unwrap_or(config.carrier_frequency)
}

#[allow(dead_code)]
fn _parse_profile_id(input: &str) -> Result<ProfileId, IrisError> {
    ProfileId::parse(input)
}
