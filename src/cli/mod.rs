use crate::config::{AppConfig, ConfigStore, default_profile_root, default_state_dir};
use crate::daemon;
use crate::errors::IrisError;
use crate::ir::{
    DryRunTransmitter, IrSignal, IrTransmitter, RppalReceiver, RppalTransmitter,
    build_nec_raw32_pulses,
};
use crate::profiles::{ProfileId, ProfileStore};
use crate::scan::{ScanSession, TerminalInput, prompt_session_name, run_interactive_session};
use crate::server::{self, RegisteredDevice};
use crate::update::{self, UpdateOptions};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
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
    Device {
        #[command(subcommand)]
        command: DeviceCommands,
    },
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
    Debug {
        #[command(subcommand)]
        command: DebugCommands,
    },
    /// Learn commands from an active-low demodulated IR receiver.
    Scan(ScanArgs),
    /// Check for and install the latest stable ARM64 release.
    Update(UpdateArgs),
    Serve(OptionalDeviceArgs),
    Status,
}

#[derive(Debug, Args)]
pub struct SendArgs {
    pub command: String,
    #[arg(long)]
    pub repeat: Option<u32>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub device: Option<String>,
}

#[derive(Debug, Args)]
pub struct DeviceArgs {
    pub brand: String,
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Debug, Args)]
pub struct OptionalDeviceArgs {
    pub brand: Option<String>,
    #[arg(long, requires = "brand")]
    pub model: Option<String>,
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Name for the learned profile and output files.
    #[arg(long)]
    pub name: Option<String>,
    /// Directory in which to create the session log and learned profile.
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Type of learned device profile.
    #[arg(long, default_value = "tv", value_parser = ["tv", "fan"])]
    pub device_type: String,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Only check whether a newer release is available.
    #[arg(long)]
    pub check: bool,
    /// Replace installed profiles with the profiles from the release.
    #[arg(long)]
    pub replace_profiles: bool,
}

#[derive(Debug, Subcommand)]
pub enum ListCommands {
    Brands,
    Models { brand: String },
}

#[derive(Debug, Subcommand)]
pub enum DeviceCommands {
    Add {
        id: String,
        profile: String,
        #[arg(long)]
        name: Option<String>,
    },
    List,
    Show {
        id: String,
    },
    Remove {
        id: String,
    },
    Use {
        id: String,
    },
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
    Start(OptionalDeviceArgs),
    Stop,
}

#[derive(Debug, Subcommand)]
pub enum HomeAssistantCommands {
    Setup,
}

#[derive(Debug, Subcommand)]
pub enum DebugCommands {
    /// Send a 32-bit NEC frame exactly as supplied, least-significant bit first.
    SendNecRaw32 { value: String },
    /// Enable a continuous 38 kHz carrier for an inspection interval in seconds.
    Carrier {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        duration: u64,
    },
}

pub async fn run(cli: Cli) -> Result<(), IrisError> {
    let config_store = ConfigStore::from_environment()?;
    let profile_store = ProfileStore::new(default_profile_root());

    match cli.command {
        Commands::Start(args) => start_profile(&config_store, &profile_store, &args),
        Commands::Send(args) => send_command(&config_store, &profile_store, args),
        Commands::Device { command } => device_command(&config_store, &profile_store, command),
        Commands::List { command } => list_command(&profile_store, command),
        Commands::Profile { command } => profile_command(&profile_store, command),
        Commands::Config { command } => config_command(&config_store, command),
        Commands::Daemon { command } => daemon_command(&config_store, &profile_store, command),
        Commands::HomeAssistant { command } => home_assistant_command(&config_store, command),
        Commands::Debug { command } => debug_command(&config_store, command),
        Commands::Scan(args) => scan_command(&config_store, args),
        Commands::Update(args) => update_command(args),
        Commands::Serve(args) => serve(&config_store, &profile_store, args).await,
        Commands::Status => status(&config_store),
    }
}

fn update_command(args: UpdateArgs) -> Result<(), IrisError> {
    let result = update::run(UpdateOptions {
        check_only: args.check,
        replace_profiles: args.replace_profiles,
        state_dir: default_state_dir()?,
    })?;

    match result {
        update::UpdateResult::UpToDate { installed, latest } => {
            println!("IRIS is up to date ({installed}; latest stable release: {latest})");
        }
        update::UpdateResult::Available { installed, latest } => {
            println!("Update available: {installed} -> {latest}");
        }
        update::UpdateResult::Installed {
            installed,
            profiles,
            daemon_restarted,
        } => {
            println!("Updated IRIS to {installed}");
            println!("Profiles {profiles}");
            if daemon_restarted {
                println!("Restarted IRIS daemon");
            }
        }
    }
    Ok(())
}

fn debug_command(config_store: &ConfigStore, command: DebugCommands) -> Result<(), IrisError> {
    const NEC_CARRIER_FREQUENCY: u32 = 38_000;

    let config = config_store.load()?;
    match command {
        DebugCommands::SendNecRaw32 { value } => {
            let data = parse_hex_u32(&value)?;
            let tx = RppalTransmitter::new(config.gpio_pin, NEC_CARRIER_FREQUENCY)?;
            tx.send_with_frequency(
                IrSignal::Raw {
                    frequency: NEC_CARRIER_FREQUENCY,
                    pulses: build_nec_raw32_pulses(data),
                },
                1,
                NEC_CARRIER_FREQUENCY,
            )
        }
        DebugCommands::Carrier { duration } => {
            let tx = RppalTransmitter::new(config.gpio_pin, NEC_CARRIER_FREQUENCY)?;
            tx.send_carrier(Duration::from_secs(duration))
        }
    }
}

fn parse_hex_u32(value: &str) -> Result<u32, IrisError> {
    let hex = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    u32::from_str_radix(hex, 16).map_err(|_| IrisError::InvalidHex {
        value: value.to_string(),
    })
}

fn scan_command(config_store: &ConfigStore, args: ScanArgs) -> Result<(), IrisError> {
    let requested_name = match args.name {
        Some(name) => name,
        None => prompt_session_name()?,
    };
    let output_directory = match args.path {
        Some(path) => path,
        None => std::env::current_dir()?,
    };
    let config = config_store.load()?;
    let mut receiver = RppalReceiver::new(config.receiver_gpio_pin, config.carrier_frequency)?;
    let mut session =
        ScanSession::new(&requested_name, output_directory, config.carrier_frequency)?;
    session.set_device_type(&args.device_type)?;
    let mut input = TerminalInput::new()?;
    let mut stdout = std::io::stdout().lock();
    let profile_path =
        run_interactive_session(&mut receiver, &mut input, &mut stdout, &mut session)?;
    drop(stdout);
    println!(
        "Wrote {} accepted command(s) to {} and {}",
        session.accepted_count(),
        session.log_path().display(),
        profile_path.display()
    );
    Ok(())
}

fn start_profile(
    config_store: &ConfigStore,
    profile_store: &ProfileStore,
    args: &DeviceArgs,
) -> Result<(), IrisError> {
    let loaded = profile_store.load_brand_model(&args.brand, args.model.as_deref())?;
    let mut config = config_store.load()?;
    config.upsert_legacy_default(loaded.id());
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
    let device = config.device(args.device.as_deref())?;
    let profile = profile_store.load(&device.profile)?;
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

fn device_command(
    config_store: &ConfigStore,
    profile_store: &ProfileStore,
    command: DeviceCommands,
) -> Result<(), IrisError> {
    let mut config = config_store.load()?;
    match command {
        DeviceCommands::Add { id, profile, name } => {
            let loaded = profile_store.load(&profile)?;
            let name = name.unwrap_or_else(|| format!("{} {}", loaded.brand, loaded.model));
            config.add_device(&id, name, loaded.id())?;
            config_store.save(&config)?;
            println!("Added device {id}");
        }
        DeviceCommands::List => {
            for device in &config.devices {
                let marker = if config.default_device.as_deref() == Some(device.id.as_str()) {
                    "*"
                } else {
                    " "
                };
                println!(
                    "{marker} {}\t{}\t{}",
                    device.id, device.name, device.profile
                );
            }
        }
        DeviceCommands::Show { id } => {
            let device = config.device(Some(&id))?;
            println!("id = {}", device.id);
            println!("name = {}", device.name);
            println!("profile = {}", device.profile);
        }
        DeviceCommands::Remove { id } => {
            config.remove_device(&id)?;
            config_store.save(&config)?;
            println!("Removed device {id}");
        }
        DeviceCommands::Use { id } => {
            config.use_device(&id)?;
            config_store.save(&config)?;
            println!("Default device is now {id}");
        }
    }
    Ok(())
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
            println!("bridge_id = {}", config.device_id.as_deref().unwrap_or(""));
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
            if let Some(brand) = args.brand {
                let loaded = profile_store.load_brand_model(&brand, args.model.as_deref())?;
                let mut config = config_store.load()?;
                config.upsert_legacy_default(loaded.id());
                config_store.save(&config)?;
            }
            let pid = daemon::start(&state_dir)?;
            println!("Started IRIS daemon with PID {pid}");
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
    args: OptionalDeviceArgs,
) -> Result<(), IrisError> {
    let mut config = config_store.load()?;
    if let Some(brand) = args.brand {
        let loaded = profile_store.load_brand_model(&brand, args.model.as_deref())?;
        config.upsert_legacy_default(loaded.id());
        config_store.save(&config)?;
    }
    let devices = load_registered_devices(profile_store, &config)?;
    let frequency = config.carrier_frequency;
    let transmitter = Arc::new(RppalTransmitter::new(config.gpio_pin, frequency)?);
    server::serve(devices, config, transmitter).await
}

fn load_registered_devices(
    profile_store: &ProfileStore,
    config: &AppConfig,
) -> Result<Vec<RegisteredDevice>, IrisError> {
    if config.devices.is_empty() {
        return Err(IrisError::DefaultDeviceMissing);
    }
    config
        .devices
        .iter()
        .map(|device| {
            Ok(RegisteredDevice {
                config: device.clone(),
                profile: profile_store.load(&device.profile)?,
            })
        })
        .collect()
}

fn status(config_store: &ConfigStore) -> Result<(), IrisError> {
    let config = config_store.load()?;
    println!(
        "active_profile = {}",
        config.active_profile.as_deref().unwrap_or("<none>")
    );
    println!("gpio_pin = {}", config.gpio_pin);
    println!("receiver_gpio_pin = {}", config.receiver_gpio_pin);
    println!(
        "default_device = {}",
        config.default_device.as_deref().unwrap_or("<none>")
    );
    println!("devices = {}", config.devices.len());
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
