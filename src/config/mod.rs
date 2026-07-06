use crate::errors::IrisError;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub gpio_pin: u8,
    #[serde(default = "default_carrier_frequency")]
    pub carrier_frequency: u32,
    #[serde(default)]
    pub active_profile: Option<String>,
    #[serde(default = "default_repeat")]
    pub default_repeat: u32,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_server_host")]
    pub server_host: String,
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    #[serde(default)]
    pub api_token: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default = "default_device_name")]
    pub device_name: String,
    #[serde(default = "default_discovery_enabled")]
    pub discovery_enabled: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gpio_pin: default_gpio_pin(),
            carrier_frequency: default_carrier_frequency(),
            active_profile: None,
            default_repeat: default_repeat(),
            log_level: default_log_level(),
            server_host: default_server_host(),
            server_port: default_server_port(),
            api_token: None,
            device_id: None,
            device_name: default_device_name(),
            discovery_enabled: default_discovery_enabled(),
        }
    }
}

fn default_gpio_pin() -> u8 {
    18
}

fn default_carrier_frequency() -> u32 {
    38_000
}

fn default_repeat() -> u32 {
    1
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_server_host() -> String {
    "127.0.0.1".to_string()
}

fn default_server_port() -> u16 {
    8787
}

fn default_device_name() -> String {
    "IRIS TV".to_string()
}

fn default_discovery_enabled() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    root: PathBuf,
}

impl ConfigStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn from_environment() -> Result<Self, IrisError> {
        if let Ok(path) = std::env::var("IRIS_CONFIG_DIR") {
            return Ok(Self::new(path));
        }

        let dirs = ProjectDirs::from("", "", "iris").ok_or_else(|| {
            IrisError::io(
                PathBuf::from("~/.config/iris"),
                std::io::Error::other("could not resolve user config directory"),
            )
        })?;
        Ok(Self::new(dirs.config_dir()))
    }

    pub fn path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn load(&self) -> Result<AppConfig, IrisError> {
        let path = self.path();
        if !path.exists() {
            return Ok(AppConfig::default());
        }

        let raw = std::fs::read_to_string(&path).map_err(|source| IrisError::io(&path, source))?;
        toml::from_str(&raw).map_err(|source| IrisError::InvalidConfigToml { path, source })
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), IrisError> {
        std::fs::create_dir_all(&self.root).map_err(|source| IrisError::io(&self.root, source))?;
        let path = self.path();
        let raw = toml::to_string_pretty(config)?;
        std::fs::write(&path, raw).map_err(|source| IrisError::io(path, source))
    }

    pub fn ensure_device_id(&self) -> Result<String, IrisError> {
        let mut config = self.load()?;
        if let Some(id) = config.device_id.as_deref().filter(|id| !id.is_empty()) {
            return Ok(id.to_string());
        }

        let id = format!("iris-{}", uuid::Uuid::new_v4().simple());
        config.device_id = Some(id.clone());
        self.save(&config)?;
        Ok(id)
    }

    pub fn prepare_home_assistant(&self) -> Result<AppConfig, IrisError> {
        let mut config = self.load()?;
        if config.device_id.as_deref().unwrap_or("").is_empty() {
            config.device_id = Some(format!("iris-{}", uuid::Uuid::new_v4().simple()));
        }
        if config.api_token.as_deref().unwrap_or("").is_empty() {
            config.api_token = Some(generate_api_token());
        }
        if config.device_name.trim().is_empty() {
            config.device_name = default_device_name();
        }
        config.server_host = "0.0.0.0".to_string();
        config.server_port = default_server_port();
        config.discovery_enabled = true;
        self.save(&config)?;
        Ok(config)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<AppConfig, IrisError> {
        let mut config = self.load()?;
        match key {
            "gpio_pin" => {
                config.gpio_pin = value.parse().map_err(|_| IrisError::InvalidConfigKey {
                    key: key.to_string(),
                })?;
            }
            "carrier_frequency" => {
                config.carrier_frequency =
                    value.parse().map_err(|_| IrisError::InvalidConfigKey {
                        key: key.to_string(),
                    })?;
            }
            "active_profile" => config.active_profile = Some(value.to_string()),
            "default_repeat" => {
                config.default_repeat = value.parse().map_err(|_| IrisError::InvalidConfigKey {
                    key: key.to_string(),
                })?;
            }
            "log_level" => config.log_level = value.to_string(),
            "server_host" => config.server_host = value.to_string(),
            "server_port" => {
                config.server_port = value.parse().map_err(|_| IrisError::InvalidConfigKey {
                    key: key.to_string(),
                })?;
            }
            "api_token" => {
                config.api_token = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "device_id" => {
                config.device_id = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "device_name" => config.device_name = value.to_string(),
            "discovery_enabled" => {
                config.discovery_enabled =
                    value.parse().map_err(|_| IrisError::InvalidConfigKey {
                        key: key.to_string(),
                    })?;
            }
            _ => {
                return Err(IrisError::InvalidConfigKey {
                    key: key.to_string(),
                });
            }
        }
        self.save(&config)?;
        Ok(config)
    }
}

fn generate_api_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub fn default_profile_root() -> PathBuf {
    if let Ok(path) = std::env::var("IRIS_PROFILE_DIR") {
        return PathBuf::from(path);
    }

    resolve_profile_root(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        Path::new("/usr/local/share/iris/profiles"),
    )
}

pub fn resolve_profile_root(current_dir: &Path, installed_profiles: &Path) -> PathBuf {
    let local_profiles = current_dir.join("profiles");
    if local_profiles.exists() {
        local_profiles
    } else {
        installed_profiles.to_path_buf()
    }
}

pub fn default_state_dir() -> Result<PathBuf, IrisError> {
    if let Ok(path) = std::env::var("IRIS_STATE_DIR") {
        return Ok(PathBuf::from(path));
    }

    let dirs = ProjectDirs::from("", "", "iris").ok_or_else(|| {
        IrisError::io(
            PathBuf::from("~/.local/share/iris"),
            std::io::Error::other("could not resolve user data directory"),
        )
    })?;
    Ok(dirs.data_dir().to_path_buf())
}
