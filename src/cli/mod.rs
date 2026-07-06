use crate::config::{AppConfig, ConfigStore, default_profile_root, default_state_dir};
use crate::daemon;
use crate::errors::IrisError;
use crate::ir::{DryRunTransmitter, IrSignal, IrTransmitter, RppalTransmitter, describe_signal};
use crate::profiles::{ProfileId, ProfileStore};
use crate::server;
use clap::{Args, Parser, Subcommand};
use std::sync::Arc;

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
    name: &'static str,
    signal: IrSignal,
    frequency: u32,
}

fn scan_command(config_store: &ConfigStore, args: ScanArgs) -> Result<(), IrisError> {
    if args.command != "power" {
        return Err(IrisError::CommandNotFound {
            command: args.command,
            profile: "scan".to_string(),
        });
    }

    let config = config_store.load()?;
    let candidates = power_scan_candidates();
    let repeat = args.repeat.unwrap_or(config.default_repeat).max(1);
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

fn power_scan_candidates() -> Vec<ScanCandidate> {
    vec![
        ScanCandidate {
            name: "tcl_nikai_power",
            signal: IrSignal::Nikai {
                data: 0x0D5F2A,
                bits: 24,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_36khz",
            signal: IrSignal::Nikai {
                data: 0x0D5F2A,
                bits: 24,
            },
            frequency: 36_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_40khz",
            signal: IrSignal::Nikai {
                data: 0x0D5F2A,
                bits: 24,
            },
            frequency: 40_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_1",
            signal: IrSignal::Nikai {
                data: 0x0CF30C,
                bits: 24,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_1_36khz",
            signal: IrSignal::Nikai {
                data: 0x0CF30C,
                bits: 24,
            },
            frequency: 36_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_1_40khz",
            signal: IrSignal::Nikai {
                data: 0x0CF30C,
                bits: 24,
            },
            frequency: 40_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_2",
            signal: IrSignal::Nikai {
                data: 0x0CFF30,
                bits: 24,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "tcl_nikai_power_alt_3",
            signal: IrSignal::Nikai {
                data: 0x0D0F2F,
                bits: 24,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_00ff_a25d",
            signal: IrSignal::Nec {
                address: 0x00FF,
                command: 0xA25D,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_00ff_45ba",
            signal: IrSignal::Nec {
                address: 0x00FF,
                command: 0x45BA,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_00ff_e21d",
            signal: IrSignal::Nec {
                address: 0x00FF,
                command: 0xE21D,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_807f_02fd",
            signal: IrSignal::Nec {
                address: 0x807F,
                command: 0x02FD,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_807f_12ed",
            signal: IrSignal::Nec {
                address: 0x807F,
                command: 0x12ED,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_807f_48b7",
            signal: IrSignal::Nec {
                address: 0x807F,
                command: 0x48B7,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_04fb_08f7",
            signal: IrSignal::Nec {
                address: 0x04FB,
                command: 0x08F7,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_04fb_0cf3",
            signal: IrSignal::Nec {
                address: 0x04FB,
                command: 0x0CF3,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_e0e0_40bf",
            signal: IrSignal::Nec {
                address: 0xE0E0,
                command: 0x40BF,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_e0e0_f20d",
            signal: IrSignal::Nec {
                address: 0xE0E0,
                command: 0xF20D,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_20df_10ef",
            signal: IrSignal::Nec {
                address: 0x20DF,
                command: 0x10EF,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_20df_23dc",
            signal: IrSignal::Nec {
                address: 0x20DF,
                command: 0x23DC,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_bf40_12ed",
            signal: IrSignal::Nec {
                address: 0xBF40,
                command: 0x12ED,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_7f80_02fd",
            signal: IrSignal::Nec {
                address: 0x7F80,
                command: 0x02FD,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_f708_fb04",
            signal: IrSignal::Nec {
                address: 0xF708,
                command: 0xFB04,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_f20d_e01f",
            signal: IrSignal::Nec {
                address: 0xF20D,
                command: 0xE01F,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_10ef_0ff0",
            signal: IrSignal::Nec {
                address: 0x10EF,
                command: 0x0FF0,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_1fe0_08f7",
            signal: IrSignal::Nec {
                address: 0x1FE0,
                command: 0x08F7,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_df20_10ef",
            signal: IrSignal::Nec {
                address: 0xDF20,
                command: 0x10EF,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_ff00_a25d",
            signal: IrSignal::Nec {
                address: 0xFF00,
                command: 0xA25D,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_0af5_10ef",
            signal: IrSignal::Nec {
                address: 0x0AF5,
                command: 0x10EF,
            },
            frequency: 38_000,
        },
        ScanCandidate {
            name: "nec_40bf_12ed",
            signal: IrSignal::Nec {
                address: 0x40BF,
                command: 0x12ED,
            },
            frequency: 38_000,
        },
    ]
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
