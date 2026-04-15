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

#[derive(Config, Serialize, Deserialize, Debug, Clone, SmartDefault)]
#[serde(default)]
pub struct ConfigRoot {
    #[config_name = "Build Mode"]
    pub build_mode: BuildMode,
    #[config_name = "Build Tool"]
    pub build_tool: BuildTool,
    #[config_name = "QEMU"]
    pub qemu: Qemu,
    #[config_name = "Bootloader"]
    pub boot_loader: Bootloader,
    #[config_name = "Kernel"]
    pub kernel: Kernel,
}

#[derive(Config, Serialize, Deserialize, Debug, Clone, SmartDefault)]
#[serde(default)]
pub struct BuildTool {
    #[config_name = "Scrollback size"]
    #[default = 1000]
    pub max_scrollback_size: usize,
    #[config_name = "ReExec when build-tool changed"]
    #[default = true]
    pub reexec: bool,
}

#[derive(Config, Serialize, Deserialize, Debug, Clone, SmartDefault)]
#[serde(default)]
pub struct Qemu {
    #[config_name = "Run qemu when build finished"]
    #[default = true]
    pub run: bool,
    #[config_name = "QEMU Guest Memory (in MB)"]
    #[default = 1024]
    pub memory: usize,
    #[config_name = "SMP Core count"]
    #[default = 8]
    pub core_count: u32,
    #[config_name = "Open GDB Server"]
    #[default = false]
    pub gdb: bool,
    #[config_name = "Open QEMU monitor console"]
    #[default = false]
    pub monitor: bool,
    #[config_name = "Enable KVM"]
    #[default = true]
    pub enable_kvm: bool,
    #[config_name = "Enable Display"]
    #[default = true]
    pub enable_display: bool,
}

#[derive(Config, Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub enum BuildMode {
    Debug,
    #[default]
    Release,
}

#[derive(Config, Serialize, SmartDefault, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Bootloader {
    #[config_name = "Any key boot"]
    pub any_key_boot: bool,

    #[config_name = "Screen resolution"]
    pub screen_resolution: ScreenResolution,

    #[config_name = "File root"]
    #[default = "\\boot"]
    pub file_root: String,

    #[config_name = "Kernel File"]
    #[default = "kernel.bin"]
    pub kernel_file: String,

    #[config_name = "Font File"]
    #[default = "kernel-font.ttf"]
    pub font_file: String,

    #[config_name = "Dwarf File"]
    #[default = "dwarf.baker"]
    pub dwarf_file: String,

    #[config_name = "Packed File"]
    #[default = "usr_bin.pak"]
    pub packed_file: String,
}

#[derive(Config, Serialize, Deserialize, SmartDefault, Debug, Clone)]
#[serde(default)]
pub struct ScreenResolution {
    #[config_name = "Width"]
    #[default = 1920]
    pub width: usize,
    #[config_name = "Height"]
    #[default = 1080]
    pub height: usize,
}

#[derive(Config, Serialize, Deserialize, SmartDefault, Debug, Clone)]
#[serde(default)]
pub struct Kernel {
    #[config_name = "Log level"]
    pub log_level: LogLevel,
    #[config_name = "Font size"]
    #[default(14)]
    pub font_size: usize,
    #[config_name = "Clock Hz rate"]
    #[default(1000)]
    pub clock_hz_rate: usize,
    #[config_name = "Stack size in pages"]
    #[default(128)]
    pub stack_size: usize,
    #[config_name = "qemu exit on panic"]
    #[default(false)]
    pub panic_exit: bool,
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
    fn into_tree(self, name: String, overwriting_name: String) -> ConfigTree;
    fn into_const_rust(&self) -> String;
    fn into_const_rust_types(&self) -> String;
    fn modifier_config<'a, C: IntoIterator<Item = &'a str>>(&mut self, config: C, value: &str) -> Result<(), Error>;
}

impl ConfigRoot {
    fn modifier_config<'a, C: IntoIterator<Item = &'a str>>(&mut self, config: C, value: &str) -> Result<(), Error> {
        let mut config = config.into_iter();
        match config.next().ok_or(Error::CannotModifyWholeGroup)? {
            "build_mode" => self.build_mode.modifier_config(config, value)?,
            "build_tool" => self.build_tool.modifier_config(config, value)?,
            unknown => return Err(Error::UnknownConfig(unknown.to_string())),
        };
        Ok(())
    }
}

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

    toml::from_str(&buf).expect("Malformed config")
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Serialization for the config failed, failed with `{0}`")]
    SerializeFailed(#[from] toml::ser::Error),
    #[error("Build dir failed, failed with `{0}`")]
    BuildDir(#[from] build::Error),
    #[error("Failed to save config, failed with `{0}`")]
    SaveConfig(#[from] io::Error),
    #[error("The selected config is not a unit value (not a group)")]
    CannotModifyWholeGroup,
    #[error("Unknown config {0}")]
    UnknownConfig(String),
    #[error("Invalid value provided {0}")]
    InvalidValue(String),
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

#[derive(Debug, Clone)]
pub enum ConfigTree {
    Group { name: String, overwriting_name: String, members: Vec<ConfigTree> },
    Value { name: String, overwriting_name: String, value: ConfigValue },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    NumberSigned(isize),
    Number(usize),
    Bool(bool),
    Text(String),
    Union { current: usize, values: Vec<String> },
}

macro_rules! cfg_value_impl {
    ($variant: ident($ty: ty), $formatter: literal, $modifier_impl: expr) => {
        impl From<$ty> for ConfigValue {
            fn from(value: $ty) -> Self {
                Self::$variant(value)
            }
        }

        impl Config for $ty {
            fn into_tree(self, name: String, overwriting_name: String) -> ConfigTree {
                ConfigTree::Value { name, overwriting_name, value: ConfigValue::$variant(self) }
            }

            fn into_const_rust(&self) -> String {
                format!($formatter, self)
            }

            fn modifier_config<'a, C: IntoIterator<Item = &'a str>>(
                &mut self,
                _config: C,
                value: &str,
            ) -> Result<(), Error> {
                $modifier_impl(self, value)
            }

            fn into_const_rust_types(&self) -> String {
                String::new()
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

macro_rules! impl_number {
    ($variant: ident($from_ty: ty => $to_type: ty)) => {
        impl From<$from_ty> for ConfigValue {
            fn from(value: $from_ty) -> Self {
                Self::$variant(value as $to_type)
            }
        }
        impl Config for $from_ty {
            fn into_tree(self, name: String, overwriting_name: String) -> ConfigTree {
                ConfigTree::Value { name, overwriting_name, value: ConfigValue::$variant(self as $to_type) }
            }
            fn into_const_rust(&self) -> String {
                format!("{}", self)
            }
            fn modifier_config<'a, C: IntoIterator<Item = &'a str>>(
                &mut self,
                _config: C,
                value: &str,
            ) -> Result<(), Error> {
                (|s: &mut $from_ty, value: &str| -> Result<(), Error> {
                    *s = value.parse().map_err(|_| Error::InvalidValue(value.to_string()))?;
                    Ok(())
                })(self, value)
            }
            fn into_const_rust_types(&self) -> String {
                String::new()
            }
        }
        impl TryFrom<ConfigTree> for $from_ty {
            type Error = ConfigTree;

            fn try_from(value: ConfigTree) -> Result<Self, Self::Error> {
                match value {
                    ConfigTree::Value { value: ConfigValue::$variant(value), .. } => Ok(value as $from_ty),
                    t => Err(t),
                }
            }
        }
    };
}

impl_number!(NumberSigned(isize => isize));
impl_number!(NumberSigned(i64 => isize));
impl_number!(NumberSigned(i32 => isize));
impl_number!(NumberSigned(i16 => isize));
impl_number!(NumberSigned(i8 => isize));

impl_number!(Number(usize => usize));
impl_number!(Number(u64 => usize));
impl_number!(Number(u32 => usize));
impl_number!(Number(u16 => usize));
impl_number!(Number(u8 => usize));

cfg_value_impl!(Bool(bool), "{}", |s: &mut bool, value: &str| -> Result<(), Error> {
    *s = match value {
        "true" => true,
        "false" => false,
        invalid => return Err(Error::InvalidValue(invalid.to_string())),
    };
    Ok(())
});

cfg_value_impl!(Text(String), "r\"{}\"", |s: &mut String, value: &str| -> Result<(), Error> {
    *s = value.to_string();
    Ok(())
});

impl Display for ConfigValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NumberSigned(value) => write!(f, "{value}"),
            Self::Number(value) => write!(f, "{value}"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Text(value) => write!(f, "{value}"),
            Self::Union { current, values, .. } => write!(f, "{}", &values[*current]),
        }
    }
}
