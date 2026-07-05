use crate::config::{AppConfig, ConfigStore, default_profile_root, default_state_dir};
use crate::daemon;
use crate::errors::IrisError;
use crate::ir::{DryRunTransmitter, IrTransmitter, RppalTransmitter};
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
    Start {
        profile: String,
    },
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
    Serve {
        profile: String,
    },
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
    Start { profile: String },
    Stop,
}

pub async fn run(cli: Cli) -> Result<(), IrisError> {
    let config_store = ConfigStore::from_environment()?;
    let profile_store = ProfileStore::new(default_profile_root());

    match cli.command {
        Commands::Start { profile } => start_profile(&config_store, &profile_store, &profile),
        Commands::Send(args) => send_command(&config_store, &profile_store, args),
        Commands::List { command } => list_command(&profile_store, command),
        Commands::Profile { command } => profile_command(&profile_store, command),
        Commands::Config { command } => config_command(&config_store, command),
        Commands::Daemon { command } => daemon_command(&config_store, &profile_store, command),
        Commands::Serve { profile } => serve(&config_store, &profile_store, &profile).await,
        Commands::Status => status(&config_store),
    }
}

fn start_profile(
    config_store: &ConfigStore,
    profile_store: &ProfileStore,
    profile: &str,
) -> Result<(), IrisError> {
    let loaded = profile_store.load(profile)?;
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

fn daemon_command(
    config_store: &ConfigStore,
    profile_store: &ProfileStore,
    command: DaemonCommands,
) -> Result<(), IrisError> {
    let state_dir = default_state_dir()?;
    match command {
        DaemonCommands::Start { profile } => {
            let loaded = profile_store.load(&profile)?;
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
    profile: &str,
) -> Result<(), IrisError> {
    let loaded = profile_store.load(profile)?;
    let mut config = config_store.load()?;
    config.active_profile = Some(loaded.id());
    config_store.save(&config)?;
    let frequency = effective_frequency(&config, &loaded);
    let transmitter = Arc::new(RppalTransmitter::new(config.gpio_pin, frequency)?);
    server::serve(loaded, config, transmitter).await
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
