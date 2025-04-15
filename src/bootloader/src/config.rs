use alloc::format;
use boot_cfg_parser::toml::parser::TomlValue;
use bootbridge::KernelConfig;

use crate::boot_services::LoaderFile;

/// A wrapper for the kernel config file,
/// The underlying config format may change so we need an abstraction
pub struct BootConfig<'a> {
    file_root: &'a str,
    kernel_file: &'a str,
    font_file: &'a str,
    dwarf_file: &'a str,
    screen_resolution: (usize, usize),
    any_key_boot: bool,
    font_size: usize,
}

impl<'a> BootConfig<'a> {
    // TODO: Change the kernel config format to something else that is not toml
    pub fn parse(toml: &'a TomlValue) -> Self {
        let file_root: &str = toml
            .get("file_root")
            .expect("file root not found in the info file")
            .as_string()
            .expect("kernel");
        let kernel_file: &str = toml
            .get("kernel_file")
            .expect("No kernel file found in the info file")
            .as_string()
            .expect("Kernel file is not a string value in file info");
        let font_file: &str = toml
            .get("font_file")
            .expect("No font file found in the info file")
            .as_string()
            .expect("Font file is not a string value in file info");
        let dwarf_file: &str = toml
            .get("dwarf_file")
            .expect("No font file found in the info file")
            .as_string()
            .expect("Font file is not a string value in file info");
        let resolution = toml
            .get("screen_resolution")
            .expect("screen_resolution not found in the config file");
        let width = resolution
            .get("width")
            .expect("width not found in the config file")
            .as_integer()
            .expect("width is not an integer") as usize;
        let height = resolution
            .get("height")
            .expect("height not found in the config file")
            .as_integer()
            .expect("height is not an integer") as usize;
        let any_key_boot = toml
            .get("any_key_boot")
            .expect("any_key_boot boot config not found")
            .as_bool()
            .expect("any_key_boot is not a boolean");
        let font_size = toml
            .get("kernel_config")
            .expect("kernel_config not found")
            .get("font_size")
            .expect("font_size not found in the config file")
            .as_integer()
            .expect("font_size is not an integer") as usize;
        Self {
            file_root,
            kernel_file,
            font_file,
            dwarf_file,
            screen_resolution: (width, height),
            any_key_boot,
            font_size,
        }
    }

    pub fn kernel_file(&self) -> LoaderFile {
        LoaderFile::new(&format!(
            "{root}\\{file}",
            root = self.file_root,
            file = self.kernel_file
        ))
    }

    pub fn font_file(&self) -> LoaderFile {
        LoaderFile::new(&format!(
            "{root}\\{file}",
            root = self.file_root,
            file = self.font_file
        ))
    }

    pub fn dwarf_file(&self) -> LoaderFile {
        LoaderFile::new(&format!(
            "{root}\\{file}",
            root = self.file_root,
            file = self.dwarf_file
        ))
    }

    pub fn screen_resolution(&self) -> (usize, usize) {
        self.screen_resolution
    }

    pub fn any_key_boot(&self) -> bool {
        self.any_key_boot
    }

    pub fn kernel_config(&self) -> KernelConfig {
        KernelConfig {
            font_pixel_size: self.font_size,
        }
    }
}
