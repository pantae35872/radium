use std::fmt::Display;

use build_tool_proc::Config;
use smart_default::SmartDefault;

#[derive(Config, Debug, Clone, Default)]
pub struct ConfigRoot {
    #[config_name = "Build Mode"]
    pub build_mode: BuildMode,
    #[config_name = "Bootloader"]
    pub boot_loader: Bootloader,
    #[config_name = "Kernel"]
    pub kernel: Kernel,
}

#[derive(Config, Debug, Clone, Copy, Default)]
pub enum BuildMode {
    #[default]
    Debug,
    Release,
}

#[derive(Config, Debug, Clone, Default)]
pub struct Bootloader {
    #[config_name = "Any key boot"]
    pub any_key_boot: bool,
    #[config_name = "Kernel File"]
    pub kernel_file: String,
}

#[derive(Config, Debug, Clone, SmartDefault)]
pub struct Kernel {
    #[config_name = "Log level"]
    pub log_level: LogLevel,
    #[config_name = "Font size"]
    #[default(14)]
    pub font_size: i32,
}

#[derive(Config, Clone, Copy, Debug, Default)]
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
