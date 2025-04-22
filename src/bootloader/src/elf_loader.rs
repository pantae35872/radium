use core::ptr::write_bytes;

use santa::{Elf, ProgramType, SectionHeaderFlags};
use uefi::table::boot::{AllocateType, MemoryType};
use uefi_services::{println, system_table};

pub fn load_elf(buffer: &'static [u8]) -> (u64, u64, u64, Elf<'static>) {
    let elf = Elf::new(buffer).expect("Failed to create elf file from the kernel file buffer");
    let mut max_alignment: u64 = 4096;
    let mut mem_min: u64 = u64::MAX;
    let mut mem_max: u64 = 0;

    elf.program_header_iter()
        .filter(|e| e.segment_type() == ProgramType::Load)
        .for_each(|e| println!("{e:?}"));

    for header in elf.program_header_iter() {
        if header.segment_type() != ProgramType::Load {
            continue;
        }

        if max_alignment < header.alignment() {
            max_alignment = header.alignment();
        }

        let mut header_begin = header.vaddr().as_u64();
        let mut header_end = header.vaddr().as_u64() + header.memsize() + max_alignment - 1;

        header_begin &= !(max_alignment - 1);
        header_end &= !(max_alignment - 1);

        if header_begin < mem_min {
            mem_min = header_begin;
        }
        if header_end > mem_max {
            mem_max = header_end;
        }
    }

    let max_memory_needed = mem_max - mem_min;
    let page_count: usize = {
        let padding = mem_min & 0x0fff;
        let total_bytes = max_memory_needed + padding;
        (1 + (total_bytes >> 12)) as usize
    };

    let program_ptr = match system_table().boot_services().allocate_pages(
        AllocateType::Address(mem_min),
        MemoryType::RUNTIME_SERVICES_CODE,
        page_count,
    ) {
        Ok(ptr) => ptr as *mut u8,
        Err(err) => {
            panic!("Failed to allocate memory for the kernel {:?}", err);
        }
    };

    unsafe {
        write_bytes(program_ptr, 0, max_memory_needed as usize);
    }

    for header in elf.program_header_iter() {
        if header.segment_type() != ProgramType::Load {
            continue;
        }

        let relative_offset = header.vaddr().as_u64() - mem_min;

        let dst = program_ptr as u64 + relative_offset;
        let src = buffer.as_ptr() as u64 + header.offset();
        let len = header.filesize();

        unsafe {
            core::ptr::copy(src as *const u8, dst as *mut u8, len as usize);
        }
    }

    let entry_point = program_ptr as u64 + (elf.entry_point() - mem_min);

    return (entry_point, mem_min, mem_max, elf);
}
