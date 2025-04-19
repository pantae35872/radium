use core::sync::atomic::Ordering;

use bootbridge::BootBridge;
use spin::Once;
use x86_64::{
    instructions::{self, interrupts},
    registers::{
        control::Cr3,
        segmentation::{Segment, CS},
    },
    structures::gdt::SegmentSelector,
};

use crate::{
    gdt::{Descriptor, Gdt},
    hlt_loop,
    interrupt::{self, IDT, LAPICS, TIMER_COUNT},
    log,
    memory::{
        allocator::linear_allocator::LinearAllocator,
        enable_nxe_bit, enable_write_protect_bit, memory_controller,
        paging::{
            table::{DirectLevel4, Table},
            ActivePageTable, EntryFlags,
        },
        Frame, FrameAllocator, PAGE_SIZE,
    },
    serial_print, serial_println,
};

extern "C" {
    pub static __trampoline_start: u8;
    pub static __trampoline_end: u8;
    pub static early_alloc: u8;
}

fn clunky_wait(ms: usize) {
    let end_time = TIMER_COUNT.load(Ordering::Relaxed) + ms;
    while TIMER_COUNT.load(Ordering::Relaxed) < end_time {
        instructions::hlt();
    }
}

#[repr(C)]
struct SmpInitializationData {
    page_table: u32,
    _padding: u32, // Just to make it clear to me
    real_page_table: u64,
    stack: u64,
    stack_bottom: u64,
}

pub fn prepare_memory(boot_bridge: &BootBridge) {
    let trampoline_size = unsafe {
        &__trampoline_end as *const u8 as usize - &__trampoline_start as *const u8 as usize
    };

    unsafe {
        core::ptr::write_bytes(&early_alloc as *const u8 as *mut u8, 0, 4096 * 64);
    }

    // We reuse the early boot memory we used to bootstrap this core
    // We know it's below 4GB range because our kernel is at 1M
    let mut boot_alloc =
        unsafe { LinearAllocator::new_custom(&early_alloc as *const u8 as usize, 4096 * 64) };

    let p4_table = boot_alloc
        .allocate_frame()
        .expect("Failed to allocate frame for temporary early boot");
    let mut bootstrap_table = unsafe {
        ActivePageTable::<DirectLevel4>::new_custom(
            p4_table.start_address().as_u64() as *mut Table<DirectLevel4>
        )
    };

    boot_bridge.kernel_elf().map_self(|start, end, flags| {
        bootstrap_table.identity_map_range(
            start.into(),
            end.into(),
            EntryFlags::from_elf_program_flags(&flags),
            &mut boot_alloc,
        )
    });

    bootstrap_table.identity_map_range(
        Frame::containing_address(0x7000),
        Frame::containing_address(0x8000),
        EntryFlags::WRITABLE | EntryFlags::PRESENT,
        &mut boot_alloc,
    );

    memory_controller().lock().ident_map(
        trampoline_size as u64 + PAGE_SIZE,
        0x7000,
        EntryFlags::WRITABLE | EntryFlags::PRESENT,
    );
    unsafe {
        core::ptr::copy(
            &__trampoline_start as *const u8,
            0x8000 as *mut u8,
            trampoline_size,
        )
    };

    let stack = memory_controller()
        .lock()
        .alloc_stack(8)
        .expect("Failed to allocate stack for ap");

    let data = SmpInitializationData {
        page_table: p4_table.start_address().as_u64() as u32,
        _padding: 0,
        real_page_table: Cr3::read().0.start_address().as_u64(),
        stack: stack.top(),
        stack_bottom: stack.bottom(),
    };

    log!(Info, "AP Bootstrap page table at {:#x}", data.page_table);
    log!(Info, "AP Bootstrap stack at {:#x}", data.stack);

    unsafe {
        core::ptr::copy(
            &data as *const SmpInitializationData,
            0x7000 as *mut SmpInitializationData,
            1,
        );
    }
}

static GDT: Once<Gdt> = Once::new();

#[no_mangle]
pub extern "C" fn ap_startup() -> ! {
    enable_write_protect_bit();
    let mut code_selector = SegmentSelector(0);
    let gdt = GDT.call_once(|| {
        let mut gdt = Gdt::new();
        code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
        gdt
    });
    gdt.load();
    unsafe {
        CS::set_reg(code_selector);
    }

    log!(Info, "Hello from cpu 1");

    hlt_loop();
}

pub fn init(boot_bridge: &BootBridge) {
    prepare_memory(boot_bridge);

    interrupts::without_interrupts(|| LAPICS.get().unwrap().lock().send_init_ipi(1));

    clunky_wait(10);
    for _ in 0..2 {
        interrupts::without_interrupts(|| LAPICS.get().unwrap().lock().send_startup_ipi(1));
        clunky_wait(1);
    }
}
