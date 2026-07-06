use crate::errors::IrisError;
use crate::ir::IrSignal;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileId {
    pub brand: String,
    pub model: String,
}

impl ProfileId {
    pub fn from_brand_model(brand: &str, model: Option<&str>) -> Self {
        Self {
            brand: slug(brand),
            model: model.map(slug).unwrap_or_else(|| "generic".to_string()),
        }
    }

    pub fn parse(input: &str) -> Result<Self, IrisError> {
        let normalized = input.trim().replace('\\', "/");
        let parts: Vec<&str> = if normalized.contains('/') {
            normalized.split('/').collect()
        } else {
            normalized.split_whitespace().collect()
        };

        if parts.len() < 2 {
            return Err(IrisError::ProfileNotFound {
                profile: input.to_string(),
            });
        }

        let brand = slug(parts[0]);
        let model = slug(&parts[1..].join("_"));
        Ok(Self { brand, model })
    }

    pub fn key(&self) -> String {
        format!("{}/{}", self.brand, self.model)
    }

    pub fn relative_path(&self) -> PathBuf {
        PathBuf::from("tv")
            .join(&self.brand)
            .join(format!("{}.toml", self.model))
    }
}

fn slug(input: &str) -> String {
    input
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub brand: String,
    pub model: String,
    pub device_type: String,
    pub carrier_frequency: Option<u32>,
    pub protocol: Option<String>,
    pub commands: BTreeMap<String, CommandDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CommandDefinition {
    Nec { address: String, command: String },
    Raw { frequency: u32, pulses: Vec<u32> },
}

impl Profile {
    pub fn from_toml_str(raw: &str) -> Result<Self, IrisError> {
        toml::from_str(raw).map_err(IrisError::InvalidProfileTomlString)
    }

    pub fn id(&self) -> String {
        format!("{}/{}", slug(&self.brand), slug(&self.model))
    }

    pub fn signal_for(&self, command_name: &str) -> Result<IrSignal, IrisError> {
        let definition =
            self.commands
                .get(command_name)
                .ok_or_else(|| IrisError::CommandNotFound {
                    command: command_name.to_string(),
                    profile: self.id(),
                })?;
        definition.to_signal()
    }
}

impl CommandDefinition {
    pub fn to_signal(&self) -> Result<IrSignal, IrisError> {
        match self {
            CommandDefinition::Nec { address, command } => Ok(IrSignal::Nec {
                address: parse_hex_u16(address)?,
                command: parse_hex_u16(command)?,
            }),
            CommandDefinition::Raw { frequency, pulses } => Ok(IrSignal::Raw {
                frequency: *frequency,
                pulses: pulses.clone(),
            }),
        }
    }
}

fn parse_hex_u16(value: &str) -> Result<u16, IrisError> {
    let trimmed = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or(value.trim());
    u16::from_str_radix(trimmed, 16).map_err(|_| IrisError::InvalidHex {
        value: value.to_string(),
    })
}

#[derive(Debug, Clone)]
pub struct ProfileStore {
    root: PathBuf,
}

impl ProfileStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn load(&self, input: &str) -> Result<Profile, IrisError> {
        let id = ProfileId::parse(input)?;
        self.load_id(&id)
    }

    pub fn load_brand_model(&self, brand: &str, model: Option<&str>) -> Result<Profile, IrisError> {
        let id = ProfileId::from_brand_model(brand, model);
        self.load_id(&id)
    }

    fn load_id(&self, id: &ProfileId) -> Result<Profile, IrisError> {
        let path = self.root.join(id.relative_path());
        if !path.exists() {
            return Err(IrisError::ProfileNotFound { profile: id.key() });
        }
        let raw = std::fs::read_to_string(&path).map_err(|source| IrisError::io(&path, source))?;
        toml::from_str(&raw).map_err(|source| IrisError::InvalidProfileToml { path, source })
    }

    pub fn list_brands(&self) -> Result<Vec<String>, IrisError> {
        let tv_root = self.root.join("tv");
        if !tv_root.exists() {
            return Ok(Vec::new());
        }
        let mut brands = Vec::new();
        for entry in
            std::fs::read_dir(&tv_root).map_err(|source| IrisError::io(&tv_root, source))?
        {
            let entry = entry.map_err(IrisError::IoPlain)?;
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                brands.push(name.to_string());
            }
        }
        brands.sort();
        Ok(brands)
    }

    pub fn list_models(&self, brand: &str) -> Result<Vec<String>, IrisError> {
        let brand_root = self.root.join("tv").join(slug(brand));
        if !brand_root.exists() {
            return Err(IrisError::ProfileNotFound {
                profile: brand.to_string(),
            });
        }
        let mut models = Vec::new();
        for entry in
            std::fs::read_dir(&brand_root).map_err(|source| IrisError::io(&brand_root, source))?
        {
            let entry = entry.map_err(IrisError::IoPlain)?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("toml")
                && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
            {
                models.push(stem.to_string());
            }
        }
        models.sort();
        Ok(models)
    }
}
