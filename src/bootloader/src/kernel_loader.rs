use alloc::format;
use boot_cfg_parser::toml::parser::TomlValue;
use bootbridge::{BootBridgeBuilder, KernelConfig};
use uefi::table::{cfg::ConfigTableEntry, Boot, SystemTable};

use crate::{boot_services::read_file, elf_loader::load_elf};

pub fn find_rsdp(config_table: &[ConfigTableEntry]) -> Option<u64> {
    config_table
        .iter()
        .find(|e| {
            matches!(
                &e.guid.to_ascii_hex_lower(),
                b"8868e871-e4f1-11d3-bc22-0080c73c8881"
            )
        })
        .or_else(|| {
            config_table.iter().find(|e| {
                matches!(
                    &e.guid.to_ascii_hex_lower(),
                    b"eb9d2d30-2d88-11d3-9a16-0090273fc14d"
                )
            })
        })
        .map(|e| e.address as u64)
}

pub fn load_kernel(
    system_table: &mut SystemTable<Boot>,
    boot_bridge: &mut BootBridgeBuilder<impl Fn(usize) -> *mut u8>,
    config: &TomlValue,
) -> u64 {
    let kernel_file: &str = config
        .get("kernel_file")
        .expect("No kernel file found in info file")
        .as_string()
        .expect("Kernel file is not a string value in file info");
    let kernel_font_file: &str = config
        .get("font_file")
        .expect("No font file found in info file")
        .as_string()
        .expect("Font file is not a string value in file info");

    let kernel_font_buffer = read_file(system_table, &format!("\\boot\\{}", kernel_font_file));
    let kernel_buffer = read_file(system_table, &format!("\\boot\\{}", kernel_file));

    let (entrypoint, _kern_start, _kern_end, elf) = load_elf(system_table, kernel_buffer);
    let font_size = config
        .get("kernel_config")
        .expect("kernel_config not found")
        .get("font_size")
        .expect("font_size not found in the config file")
        .as_integer()
        .expect("font_size is not an integer") as usize;

    boot_bridge
        .kernel_elf(elf)
        .font_data(kernel_font_buffer.as_ptr() as u64, kernel_font_buffer.len())
        .kernel_config(KernelConfig {
            font_pixel_size: font_size,
        })
        .rsdp(find_rsdp(system_table.config_table()).expect("Failed to find RSDP"));

    entrypoint
}
