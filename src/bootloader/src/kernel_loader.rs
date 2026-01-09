use bakery::DwarfBaker;
use packery::Packed;
use pager::address::PhysAddr;
use santa::Elf;
use uefi::table::{
    boot::{AllocateType, MemoryType},
    cfg::ConfigTableEntry,
};
use uefi_services::system_table;

use crate::{
    boot_services::LoaderFile,
    config::config,
    context::{InitializationContext, Stage1, Stage2},
};

pub fn find_rsdp(config_table: &[ConfigTableEntry]) -> Option<u64> {
    config_table
        .iter()
        .find(|e| matches!(&e.guid.to_ascii_hex_lower(), b"8868e871-e4f1-11d3-bc22-0080c73c8881"))
        .or_else(|| {
            config_table
                .iter()
                .find(|e| matches!(&e.guid.to_ascii_hex_lower(), b"eb9d2d30-2d88-11d3-9a16-0090273fc14d"))
        })
        .map(|e| e.address as u64)
}

pub fn load_kernel_elf(ctx: InitializationContext<Stage1>) -> InitializationContext<Stage2> {
    let elf = Elf::new(ctx.context().kernel_file).expect("Failed to create elf file from the kernel file buffer");
    let program_ptr = match system_table().boot_services().allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_CODE,
        elf.page_needed(),
    ) {
        Ok(ptr) => ptr as *mut u8,
        Err(err) => {
            panic!("Failed to allocate memory for the kernel {:?}", err);
        }
    };

    let entry = unsafe { elf.load_data(program_ptr) };

    ctx.next((entry, PhysAddr::new(program_ptr as u64), elf))
}

pub fn load_kernel_infos() -> InitializationContext<Stage1> {
    let system_table = system_table();
    let kernel_font = LoaderFile::root(config().boot_loader.font_file).raw_data_permanent();
    let kernel_file = LoaderFile::root(config().boot_loader.kernel_file).permanent();
    let dwarf_file = LoaderFile::root(config().boot_loader.dwarf_file).permanent();
    let packed_file = LoaderFile::root(config().boot_loader.packed_file).permanent();
    let rsdp = PhysAddr::new(find_rsdp(system_table.config_table()).expect("Failed to find RSDP"));
    let dwarf_file = DwarfBaker::new(dwarf_file);
    let packed_file = Packed::new(packed_file).expect("Packed file not valid");
    InitializationContext::<Stage1>::start(kernel_font, dwarf_file, packed_file, rsdp, kernel_file)
}
