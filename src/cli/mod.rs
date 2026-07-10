use crate::config::{AppConfig, ConfigStore, default_profile_root, default_state_dir};
use crate::daemon;
use crate::errors::IrisError;
use crate::ir::{DryRunTransmitter, IrTransmitter, RppalReceiver, RppalTransmitter};
use crate::profiles::{ProfileId, ProfileStore};
use crate::scan::{ScanSession, TerminalInput, prompt_session_name, run_interactive_session};
use crate::server;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
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
    /// Learn commands from an active-low demodulated IR receiver.
    Scan(ScanArgs),
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
pub struct DeviceArgs {
    pub brand: String,
    #[arg(long)]
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
        Commands::List { command } => list_command(&profile_store, command),
        Commands::Profile { command } => profile_command(&profile_store, command),
        Commands::Config { command } => config_command(&config_store, command),
        Commands::Daemon { command } => daemon_command(&config_store, &profile_store, command),
        Commands::HomeAssistant { command } => home_assistant_command(&config_store, command),
        Commands::Scan(args) => scan_command(&config_store, args),
        Commands::Serve(args) => serve(&config_store, &profile_store, &args).await,
        Commands::Status => status(&config_store),
    }
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
    println!("receiver_gpio_pin = {}", config.receiver_gpio_pin);
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
