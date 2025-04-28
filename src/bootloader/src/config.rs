use alloc::format;
use boot_cfg_parser::toml::parser::TomlValue;
use bootbridge::KernelConfig;
use uefi::table::boot::PAGE_SIZE;

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
    log_level: u64,
    early_boot_kernel_page_table_page_count: usize,
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
        let early_boot_kernel_page_table_page_count = toml.get("early_boot_kernel_page_table_page_count")
            .expect("early_boot_kernel_page_table_page_count no found in the config file (required for kernel page tables)")
            .as_integer()
            .expect("early_boot_kernel_page_table_page_count is not an interger") as usize;
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
        let kconfig = toml.get("kernel_config").expect("kernel_config not found");
        let font_size = kconfig
            .get("font_size")
            .expect("font_size not found in the config file")
            .as_integer()
            .expect("font_size is not an integer") as usize;
        let log_level = kconfig
            .get("log_level")
            .expect("font_size not found in the config file")
            .as_integer()
            .expect("font_size is not an integer") as u64;
        Self {
            file_root,
            kernel_file,
            font_file,
            dwarf_file,
            screen_resolution: (width, height),
            any_key_boot,
            font_size,
            log_level,
            early_boot_kernel_page_table_page_count,
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
            log_level: self.log_level,
        }
    }

    pub fn early_boot_kernel_page_table_byte_count(&self) -> usize {
        self.early_boot_kernel_page_table_page_count * PAGE_SIZE
    }

    pub fn early_boot_kernel_page_table_page_count(&self) -> usize {
        self.early_boot_kernel_page_table_page_count
    }
}
