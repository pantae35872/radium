use bootbridge::BootBridgeBuilder;
use uefi::table::cfg::ConfigTableEntry;
use uefi_services::system_table;

use crate::{config::BootConfig, elf_loader::load_elf};

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
    boot_bridge: &mut BootBridgeBuilder<impl Fn(usize) -> *mut u8>,
    config: &BootConfig,
) -> u64 {
    let system_table = system_table();
    let kernel_font = config.font_file().permanent();
    let kernel_file = config.kernel_file().permanent();

    let (entrypoint, _kern_start, _kern_end, elf) = load_elf(kernel_file);

    boot_bridge
        .kernel_elf(elf)
        .font_data(kernel_font.as_ptr() as u64, kernel_font.len())
        .kernel_config(config.kernel_config())
        .rsdp(find_rsdp(system_table.config_table()).expect("Failed to find RSDP"));

    entrypoint
}
