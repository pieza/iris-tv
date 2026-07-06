use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum IrisError {
    #[error("profile not found: {profile}")]
    ProfileNotFound { profile: String },

    #[error("no active profile is configured; run `iris start <brand>` first")]
    ActiveProfileMissing,

    #[error("command `{command}` does not exist in profile `{profile}`")]
    CommandNotFound { command: String, profile: String },

    #[error("invalid TOML profile `{path}`: {source}")]
    InvalidProfileToml {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("invalid profile TOML: {0}")]
    InvalidProfileTomlString(#[from] toml::de::Error),

    #[error("invalid global config `{path}`: {source}")]
    InvalidConfigToml {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("unsupported IR protocol: {protocol}")]
    UnsupportedProtocol { protocol: String },

    #[error("invalid hex value `{value}`")]
    InvalidHex { value: String },

    #[error(
        "GPIO is not available in this build; rebuild with `--features rpi-gpio` on Raspberry Pi OS"
    )]
    GpioUnavailable,

    #[error("permission denied while accessing GPIO pin {pin}; try running with GPIO permissions")]
    GpioPermissionDenied { pin: u8 },

    #[error("daemon is already running with PID {pid}")]
    DaemonAlreadyRunning { pid: u32 },

    #[error("daemon is not running")]
    DaemonNotRunning,

    #[error("failed to bind server to {addr}: {source}")]
    ServerBindFailure {
        addr: String,
        source: std::io::Error,
    },

    #[error("unauthorized API request")]
    Unauthorized,

    #[error("I/O error at `{path}`: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("I/O error: {0}")]
    IoPlain(#[from] std::io::Error),

    #[error("failed to serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("invalid configuration key `{key}`")]
    InvalidConfigKey { key: String },

    #[error("server exposure requires api_token when server_host is not loopback")]
    MissingApiTokenForNetworkBind,

    #[error("failed to advertise mDNS service: {0}")]
    Discovery(String),
}

impl IrisError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
