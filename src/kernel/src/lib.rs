#![no_std]
#![no_main]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![feature(abi_x86_interrupt)]
#![feature(ptr_internals)]
#![feature(custom_test_frameworks)]
#![feature(core_intrinsics)]
#![feature(str_from_utf16_endian)]
#![feature(pointer_is_aligned_to)]
#![feature(sync_unsafe_cell)]
#![feature(iter_next_chunk)]
#![feature(iter_array_chunks)]
#![feature(decl_macro)]
#![feature(assert_matches)]
#![feature(vec_from_fn)]
#![recursion_limit = "16384"]
#![allow(internal_features)]
#![allow(dead_code)]
#![allow(unused_macros)]
#![allow(clippy::fn_to_numeric_cast)]

#[macro_use]
extern crate bitflags;
extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate spin;

pub mod buffer;
pub mod driver;
pub mod gdt;
pub mod graphics;
pub mod initialization_context;
pub mod interrupt;
pub mod logger;
pub mod memory;
pub mod port;
pub mod print;
pub mod serial;
pub mod sync;
pub mod syscall;
pub mod userland;
pub mod utils;

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::{ffi::c_void, sync::atomic::AtomicBool};

use alloc::sync::Arc;
use bakery::DwarfBaker;
use bootbridge::{BootBridge, RawBootBridge};
use conquer_once::spin::OnceCell;
use driver::{
    acpi::{self},
    pit,
};
use graphics::BACKGROUND_COLOR;
use graphics::color::Color;
use initialization_context::{InitializationContext, Stage0};
use kernel_proc::{def_local, local_builder};
use logger::LOGGER;
use port::{Port, Port32Bit, PortWrite};
use sentinel::log;
use smp::cpu_local_avaiable;
use spin::Mutex;
use unwinding::abi::{_Unwind_Backtrace, _Unwind_GetIP, UnwindContext, UnwindReasonCode};

use crate::interrupt::CORE_ID;
use crate::userland::pipeline::CURRENT_THREAD_ID;

static DWARF_DATA: OnceCell<DwarfBaker<'static>> = OnceCell::uninit();
static STILL_INITIALIZING: AtomicBool = AtomicBool::new(true);

def_local!(pub static BOOT_BRIDGE: Arc<BootBridge>);

pub fn init(boot_bridge: *mut RawBootBridge) -> ! {
    initialize_guard!();

    let boot_bridge = BootBridge::new(boot_bridge);
    let mut stage0 = InitializationContext::<Stage0>::start(boot_bridge);
    logger::init(&stage0);
    qemu_init(&mut stage0);
    let stage1 = memory::init(stage0);
    let mut stage2 = acpi::init(stage1);
    graphics::init(&mut stage2);
    print::init(&mut stage2, Color::new(166, 173, 200), BACKGROUND_COLOR);
    let mut stage3 = smp::init(stage2);
    gdt::init_gdt(&mut stage3);
    let mut stage4 = interrupt::init(stage3);
    stage4.local_initializer(|i| {
        i.context_transformer(|builder, ctx| {
            builder.boot_bridge(Arc::new(ctx.context.take_boot_bridge().unwrap()));
        });
        i.register(|builder, ctx, _id| {
            local_builder!(builder, BOOT_BRIDGE(Arc::clone(&ctx.boot_bridge)));
        });
    });
    memory::init_local(&mut stage4);
    userland::init(&mut stage4);
    pit::init(&mut stage4);
    syscall::init(&mut stage4);
    smp::init_aps(stage4);

    //LOGGER.flush_all(&[|s| serial_print!("{s}"), |s| print!("{s}")]);

    userland::pipeline::spawn_init();
    userland::pipeline::start_scheduling();
    hlt_loop();
}

#[macro_export]
macro_rules! initialize_guard {
    () => {
        if !$crate::STILL_INITIALIZING.load(core::sync::atomic::Ordering::SeqCst) {
            panic!("Trying to call initialize function when the kernel is already initialized");
        }
    };
}

#[cfg(test)]
#[unsafe(no_mangle)]
pub extern "C" fn start(boot_info: *mut RawBootBridge) -> ! {
    init(boot_info);
}

#[inline(always)]
pub fn hlt() {
    unsafe {
        core::arch::asm!("hlt", options(nostack, preserves_flags, nomem));
    }
}

pub fn hlt_loop() -> ! {
    loop {
        hlt();
    }
}

pub trait Testable {
    fn run(&self);
}

pub const TESTING: bool = cfg!(test) | cfg!(feature = "testing");
pub const QEMU_EXIT_PANIC: bool = cfg!(feature = "panic_exit");

static PANIC_COUNT: AtomicUsize = AtomicUsize::new(0);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    interrupt::disable();
    print::DRIVER.get().inspect(|e| unsafe { e.force_unlock() });
    unsafe { serial::SERIAL1.force_unlock() };
    match PANIC_COUNT.fetch_add(1, Ordering::SeqCst) {
        0 => {}
        1 => {
            log!(Critical, "DOUBLE PANIC STOPPING...");
            LOGGER.flush_select();
            hlt_loop();
        }
        2 => {
            serial_println!("TRIPLE PANIC, theres a bug in the logger code");
            hlt_loop();
        }
        _ => hlt_loop(), // LAST CASE THERES A BUG IN THE SERIAL LOGGER
    };
    if cpu_local_avaiable() {
        let id = match CURRENT_THREAD_ID.try_borrow() {
            Ok(id) => *id,
            Err(_) => 0,
        };
        log!(Critical, "PANIC on core: {}, thread id: {id}", *CORE_ID,);
    }
    log!(Critical, "{}", info);

    log!(Info, "Backtrace:");
    struct CallbackData {
        counter: usize,
    }
    extern "C" fn callback(unwind_ctx: &UnwindContext<'_>, arg: *mut c_void) -> UnwindReasonCode {
        let data = unsafe { &mut *(arg as *mut CallbackData) };
        data.counter += 1;
        let ip = _Unwind_GetIP(unwind_ctx);
        if let Some(dwarf) = DWARF_DATA.get() {
            let (line_num, name, location) = dwarf.by_addr(ip as u64).unwrap_or((0, "unknown", "unknown"));
            log!(Info, "{:4}:{:#x} - {name}", data.counter, ip);
            log!(Info, "{:>12} at {:<30}:{:<4}", "", location, line_num);
            if name == "start" || name == "ap_startup" || name.contains("thread_trampoline") {
                UnwindReasonCode::END_OF_STACK
            } else {
                UnwindReasonCode::NO_REASON
            }
        } else {
            // Since we can't know the name if the dwarf data is not initialized we assumed end of
            // stack for safety
            UnwindReasonCode::END_OF_STACK
        }
    }
    let mut data = CallbackData { counter: 0 };
    _Unwind_Backtrace(callback, &mut data as *mut _ as _);

    LOGGER.flush_select();

    if TESTING {
        test_panic_handler(info);
    } else if QEMU_EXIT_PANIC {
        exit_qemu(QemuExitCode::Failed);
    } else {
        hlt_loop();
    }
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

pub fn test_panic_handler(_info: &PanicInfo) -> ! {
    serial_println!("[failed]");
    exit_qemu(QemuExitCode::Failed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

static QEMU_EXIT_PORT: OnceCell<Mutex<Port<Port32Bit, PortWrite>>> = OnceCell::uninit();

fn qemu_init(ctx: &mut InitializationContext<Stage0>) {
    QEMU_EXIT_PORT.init_once(|| {
        ctx.context_mut().port_allocator.allocate(0xf4).expect("FAILED TO ALLOCATE QEMU EXIT PORT").into()
    });
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    unsafe {
        QEMU_EXIT_PORT.get().unwrap().lock().write(exit_code as u32);
    }

    // Wait for qemu to exit
    hlt_loop();
}

// Apparently the order of proc macro does matter; that should be obvious now, that why is this here.
pub mod smp;
