use crate::errors::IrisError;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub gpio_pin: u8,
    pub carrier_frequency: u32,
    pub active_profile: Option<String>,
    pub default_repeat: u32,
    pub log_level: String,
    pub server_host: String,
    pub server_port: u16,
    pub api_token: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gpio_pin: 18,
            carrier_frequency: 38_000,
            active_profile: None,
            default_repeat: 1,
            log_level: "info".to_string(),
            server_host: "127.0.0.1".to_string(),
            server_port: 8787,
            api_token: None,
        }
    }
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

pub fn default_profile_root() -> PathBuf {
    if let Ok(path) = std::env::var("IRIS_PROFILE_DIR") {
        return PathBuf::from(path);
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("profiles")
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
