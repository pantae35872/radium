#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![feature(abi_x86_interrupt)]
#![feature(ptr_internals)]
#![feature(core_intrinsics)]
#![feature(str_from_utf16_endian)]
#![feature(naked_functions)]
#![feature(pointer_is_aligned_to)]
#![allow(internal_features)]
#![allow(undefined_naked_function_abi)]
#![deny(warnings)]

#[macro_use]
extern crate bitflags;

extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate spin;

pub mod driver;
pub mod filesystem;
pub mod gdt;
pub mod graphics;
pub mod interrupt;
pub mod memory;
pub mod print;
pub mod serial;
pub mod task;
pub mod userland;
pub mod utils;

use core::panic::PanicInfo;
use core::usize;

use common::boot::BootInformation;
use conquer_once::spin::OnceCell;
use memory::allocator::buddy_allocator::BuddyAllocator;
use memory::allocator::{self, HEAP_SIZE, HEAP_START};
use memory::paging::{ActivePageTable, EntryFlags, Page};
use memory::stack_allocator::{Stack, StackAllocator};
use memory::Frame;
use spin::Mutex;
use x86_64::registers::control::Cr0Flags;
use x86_64::registers::model_specific::EferFlags;
use x86_64::{PhysAddr, VirtAddr};

pub trait Testable {
    fn run(&self) -> ();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    test_panic_handler(info);
    hlt_loop();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

pub fn test_runner(_tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", _tests.len());
    for test in _tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

static MEMORY_CONTROLLER: OnceCell<Mutex<MemoryController<64>>> = OnceCell::uninit();

pub fn get_memory_controller() -> &'static Mutex<MemoryController<64>> {
    return MEMORY_CONTROLLER
        .get()
        .expect("Memory controller not initialized");
}

pub struct MemoryController<const ORDER: usize> {
    active_table: ActivePageTable,
    allocator: BuddyAllocator<'static, ORDER>,
    stack_allocator: StackAllocator,
}

impl<const ORDER: usize> MemoryController<ORDER> {
    pub fn alloc_stack(&mut self, size_in_pages: usize) -> Option<Stack> {
        self.stack_allocator
            .alloc_stack(&mut self.active_table, &mut self.allocator, size_in_pages)
    }

    pub fn map(&mut self, page: Page, flags: EntryFlags) {
        self.active_table.map(page, flags, &mut self.allocator);
    }

    pub fn map_to(&mut self, page: Page, frame: Frame, flags: EntryFlags) {
        self.active_table
            .map_to(page, frame, flags, &mut self.allocator);
    }

    pub fn allocate(&mut self, size: usize) -> Option<*mut u8> {
        return self.allocator.allocate(size);
    }

    pub fn deallocate(&mut self, ptr: *mut u8, size: usize) {
        self.allocator.dealloc(ptr, size);
    }

    pub fn get_physical(&mut self, addr: VirtAddr) -> Option<PhysAddr> {
        return self.active_table.translate(addr);
    }
}
pub fn init(information_address: *const BootInformation) {
    let boot_info = unsafe { BootInformation::from_ptr(information_address) };
    let mut allocator = unsafe { BuddyAllocator::new(boot_info.memory_map()) };
    enable_nxe_bit();
    enable_write_protect_bit();
    let active_table = memory::remap_the_kernel(&mut allocator, &boot_info);
    let stack_allocator = {
        let stack_alloc_start = Page::containing_address(HEAP_START + HEAP_SIZE);
        let stack_alloc_end = stack_alloc_start + 100;
        let stack_alloc_range = Page::range_inclusive(stack_alloc_start, stack_alloc_end);
        StackAllocator::new(stack_alloc_range)
    };
    MEMORY_CONTROLLER.init_once(|| {
        MemoryController {
            active_table,
            allocator,
            stack_allocator,
        }
        .into()
    });
    allocator::init();
    graphics::init(boot_info);
    print::init(boot_info, 0x00ff44);
    gdt::init_gdt();
    interrupt::init();
    driver::init();
    userland::init();
    x86_64::instructions::interrupts::enable();
    //    println!(
    //        r#"nothingos Copyright (C) 2024  Pantae
    //This program comes with ABSOLUTELY NO WARRANTY; for details type `show w'.
    //This is free software, and you are welcome to redistribute it
    //under certain conditions; type `show c' for details."#
    //    );
}

fn enable_write_protect_bit() {
    use x86_64::registers::control::Cr0;

    unsafe {
        let mut cr0 = Cr0::read();
        cr0.insert(Cr0Flags::WRITE_PROTECT);
        Cr0::write(cr0);
    }
}

fn enable_nxe_bit() {
    use x86_64::registers::model_specific::Efer;

    unsafe {
        let mut efer = Efer::read();
        efer.insert(EferFlags::NO_EXECUTE_ENABLE);
        Efer::write(efer);
    }
}

#[cfg(test)]
#[no_mangle]
pub extern "C" fn start(boot_info: *mut BootInformation) -> ! {
    init(boot_info);
    test_main();
    hlt_loop();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}
