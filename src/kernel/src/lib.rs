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
#![recursion_limit = "512"]
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

pub mod driver;
pub mod gdt;
pub mod graphics;
pub mod initialization_context;
pub mod interrupt;
pub mod logger;
pub mod memory;
pub mod port;
pub mod print;
pub mod scheduler;
pub mod serial;
pub mod userland;
pub mod utils;

pub mod smp;

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::{ffi::c_void, sync::atomic::AtomicBool};

use bakery::DwarfBaker;
use bootbridge::{BootBridge, RawBootBridge};
use conquer_once::spin::OnceCell;
use driver::uefi_runtime::{self};
use driver::{
    acpi::{self},
    pit,
};
use graphics::BACKGROUND_COLOR;
use graphics::color::Color;
use initialization_context::{InitializationContext, Stage0};
use logger::LOGGER;
use port::{Port, Port32Bit, PortWrite};
use scheduler::sleep;
use sentinel::log;
use smp::{ALL_AP_INITIALIZED, cpu_local_avaiable};
use spin::Mutex;
use unwinding::abi::{_Unwind_Backtrace, _Unwind_GetIP, UnwindContext, UnwindReasonCode};

use crate::interrupt::CORE_ID;
use crate::scheduler::{CURRENT_THREAD_ID, LOCAL_SCHEDULER};
use crate::smp::CTX;

static DWARF_DATA: OnceCell<DwarfBaker<'static>> = OnceCell::uninit();
static STILL_INITIALIZING: AtomicBool = AtomicBool::new(true);

pub fn init<F>(boot_bridge: *mut RawBootBridge, main_thread: F) -> !
where
    F: FnOnce() + Send + 'static,
{
    let boot_bridge = BootBridge::new(boot_bridge);
    let mut phase0 = InitializationContext::<Stage0>::start(boot_bridge);
    logger::init(&phase0);
    qemu_init(&mut phase0);
    let phase1 = memory::init(phase0);
    let mut phase2 = acpi::init(phase1);
    graphics::init(&mut phase2);
    print::init(&mut phase2, Color::new(166, 173, 200), BACKGROUND_COLOR);
    let mut phase3 = smp::init(phase2);
    gdt::init_gdt(&mut phase3);
    let mut final_phase = interrupt::init(phase3);
    scheduler::init(&mut final_phase);
    userland::init(&mut final_phase);
    pit::init(&mut final_phase);
    smp::init_aps(final_phase);

    LOCAL_SCHEDULER.inner_mut().spawn(|| {
        while !ALL_AP_INITIALIZED.load(Ordering::Relaxed) {
            sleep(1000);
        }
        sleep(1000);

        uefi_runtime::init(&mut CTX.lock());

        main_thread()
    });
    LOCAL_SCHEDULER.inner_mut().start_scheduling();

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
    init(boot_info, test_main);
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
        log!(
            Critical,
            "PANIC on core: {}, thread id: {}",
            *CORE_ID,
            *CURRENT_THREAD_ID
        );
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
            let (line_num, name, location) = dwarf
                .by_addr(ip as u64)
                .unwrap_or((0, "unknown", "unknown"));
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
        ctx.context_mut()
            .port_allocator
            .allocate(0xf4)
            .expect("FAILED TO ALLOCATE QEMU EXIT PORT")
            .into()
    });
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    unsafe {
        QEMU_EXIT_PORT.get().unwrap().lock().write(exit_code as u32);
    }

    // Wait for qemu to exit
    hlt_loop();
}
