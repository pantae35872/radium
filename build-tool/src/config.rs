use std::{
    fmt::Display,
    fs::{OpenOptions, remove_file},
    io::{self, Read, Write},
};

use build_tool_proc::Config;
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use thiserror::Error;

use crate::build::{self, make_build_dir};

pub fn load() -> ConfigRoot {
    let Some(config_path) = make_build_dir().ok().map(|p| p.join("config.toml")) else {
        return ConfigRoot::default();
    };
    if !config_path.exists() {
        return ConfigRoot::default();
    }

    let mut buf = String::new();
    let Some(_readed) =
        OpenOptions::new().read(true).open(config_path).and_then(|mut config| config.read_to_string(&mut buf)).ok()
    else {
        return ConfigRoot::default();
    };

    toml::from_str(&buf).unwrap_or_default()
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Serialization for the config failed, failed with `{0}`")]
    SerializeFailed(#[from] toml::ser::Error),
    #[error("Build dir failed, failed with `{0}`")]
    BuildDir(#[from] build::Error),
    #[error("failed to save config, failed with `{0}`")]
    SaveConfig(#[from] io::Error),
}

pub fn save(config: &ConfigRoot) -> Result<(), Error> {
    let config_path = make_build_dir().map(|p| p.join("config.toml"))?;
    let toml_string = toml::to_string_pretty(config)?;

    if config_path.exists() {
        remove_file(&config_path)?;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(config_path)
        .and_then(|mut config| config.write_all(toml_string.as_bytes()))?;

    Ok(())
}

#[derive(Config, Serialize, Deserialize, Debug, Clone, Default)]
pub struct ConfigRoot {
    #[config_name = "Build Mode"]
    pub build_mode: BuildMode,
    #[config_name = "Bootloader"]
    pub boot_loader: Bootloader,
    #[config_name = "Kernel"]
    pub kernel: Kernel,
}

#[derive(Config, Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub enum BuildMode {
    #[default]
    Debug,
    Release,
}

#[derive(Config, Serialize, Deserialize, Debug, Clone, Default)]
pub struct Bootloader {
    #[config_name = "Any key boot"]
    pub any_key_boot: bool,
    #[config_name = "Kernel File"]
    pub kernel_file: String,
}

#[derive(Config, Serialize, Deserialize, SmartDefault, Debug, Clone)]
pub struct Kernel {
    #[config_name = "Log level"]
    pub log_level: LogLevel,
    #[config_name = "Font size"]
    #[default(14)]
    pub font_size: i32,
}

#[derive(Config, Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warning,
    Error,
    Critical,
}

pub trait Config: TryFrom<ConfigTree, Error = ConfigTree> {
    fn into_tree(self, name: String) -> ConfigTree;
}

#[derive(Debug, Clone)]
pub enum ConfigTree {
    Group { name: String, members: Vec<ConfigTree> },
    Value { name: String, value: ConfigValue },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    Number(i32),
    Bool(bool),
    Text(String),
    Union { current: usize, values: Vec<String> },
}

macro_rules! cfg_value_impl {
    ($variant: ident($ty: ty)) => {
        impl From<$ty> for ConfigValue {
            fn from(value: $ty) -> Self {
                Self::$variant(value)
            }
        }

        impl Config for $ty {
            fn into_tree(self, name: String) -> ConfigTree {
                ConfigTree::Value { name, value: ConfigValue::$variant(self) }
            }
        }

        impl TryFrom<ConfigTree> for $ty {
            type Error = ConfigTree;

            fn try_from(value: ConfigTree) -> Result<Self, Self::Error> {
                match value {
                    ConfigTree::Value { value: ConfigValue::$variant(value), .. } => Ok(value),
                    t => Err(t),
                }
            }
        }
    };
}

cfg_value_impl!(Number(i32));
cfg_value_impl!(Bool(bool));
cfg_value_impl!(Text(String));

impl Display for ConfigValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(value) => write!(f, "{value}"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Text(value) => write!(f, "{value}"),
            Self::Union { current, values, .. } => write!(f, "{}", &values[*current]),
        }
    }
}
