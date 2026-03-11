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

use core::arch::asm;
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
use pager::registers::{CS, Cr0, Cr2, Cr3, Cr4, Efer, GsBase, KernelGsBase, RFlags, SS};
use port::{Port, Port32Bit, PortWrite};
use sentinel::log;
use smp::cpu_local_avaiable;
use spin::Mutex;
use unwinding::abi::{_Unwind_Backtrace, _Unwind_GetIP, UnwindContext, UnwindReasonCode};

use crate::interrupt::CORE_ID;
use crate::memory::is_stack_aligned_16;
use crate::userland::pipeline::CURRENT_THREAD_ID;

static DWARF_DATA: OnceCell<DwarfBaker<'static>> = OnceCell::uninit();
static STILL_INITIALIZING: AtomicBool = AtomicBool::new(true);

def_local!(pub static BOOT_BRIDGE: Arc<BootBridge>);

pub fn init(boot_bridge: *mut RawBootBridge) -> ! {
    initialize_guard!();

    let boot_bridge = BootBridge::new(boot_bridge);
    let mut stage0 = InitializationContext::<Stage0>::start(boot_bridge);
    logger::init();
    qemu_init(&mut stage0);
    assert!(is_stack_aligned_16(), "Unaligned stack");
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
    LOGGER.flush_all(&[|s| serial_print!("{s}"), |s| print!("{s}")]);
    smp::init_aps(stage4);

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
    let regs = RegisterDump::capture();
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
        _ => hlt_loop(),
    };

    print::DRIVER.get().inspect(|e| unsafe { e.force_unlock() });
    unsafe { serial::SERIAL1.force_unlock() };

    if cpu_local_avaiable() {
        let id = match CURRENT_THREAD_ID.try_borrow() {
            Ok(id) => *id,
            Err(_) => 0,
        };
        log!(Critical, "PANIC on core: {}, thread id: {id}", *CORE_ID,);
    }
    log!(Critical, "{}", info);

    dump_registers(regs);

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
            if name == "start" || name == "ap_startup" || name == "syscall_entry" {
                UnwindReasonCode::END_OF_STACK
            } else {
                UnwindReasonCode::NO_REASON
            }
        } else {
            log!(Info, "No backtrace");
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

#[derive(Debug, Clone, Copy)]
struct RegisterDump {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    rsp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
}

impl RegisterDump {
    fn capture() -> Self {
        let (rax, rbx, rcx, rdx, rsi, rdi, rbp, r8, r9, r10, r11, r12, r13, r14, r15): (
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
        );
        let rsp: u64;
        let rip: u64;

        unsafe {
            asm!(
                "mov {rax}, rax",
                "mov {rbx}, rbx",
                "mov {rcx}, rcx",
                "mov {rdx}, rdx",
                "mov {rsi}, rsi",
                "mov {rdi}, rdi",
                "mov {rbp}, rbp",
                "mov {r8}, r8",
                "mov {r9}, r9",
                "mov {r10}, r10",
                "mov {r11}, r11",
                "mov {r12}, r12",
                "mov {r13}, r13",
                "mov {r14}, r14",
                "mov {r15}, r15",
                rax = out(reg) rax,
                rbx = out(reg) rbx,
                rcx = out(reg) rcx,
                rdx = out(reg) rdx,
                rsi = out(reg) rsi,
                rdi = out(reg) rdi,
                rbp = out(reg) rbp,
                r8 = out(reg) r8,
                r9 = out(reg) r9,
                r10 = out(reg) r10,
                r11 = out(reg) r11,
                r12 = out(reg) r12,
                r13 = out(reg) r13,
                r14 = out(reg) r14,
                r15 = out(reg) r15,
                options(nostack, preserves_flags),
            );
            asm!("mov {0}, rsp", out(reg) rsp, options(nostack, preserves_flags));
            asm!("lea {0}, [rip + 0]", out(reg) rip, options(nostack, preserves_flags));
        }

        Self { rax, rbx, rcx, rdx, rsi, rdi, rbp, rsp, r8, r9, r10, r11, r12, r13, r14, r15, rip }
    }
}

fn dump_registers(regs: RegisterDump) {
    let rflags = RFlags::read();
    let cs = CS::read();
    let ss = SS::read();
    let cr0 = Cr0::read();
    let cr2 = Cr2::read().addr().as_u64();
    let (cr3_frame, cr3_flags) = Cr3::read();
    let cr4 = Cr4::read();
    let efer = Efer::read();
    let gs_base = GsBase::read().as_u64();
    let kgs_base = KernelGsBase::read().as_u64();

    log!(Critical, "Register dump:");
    log!(Critical, "RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}", regs.rax, regs.rbx, regs.rcx, regs.rdx);
    log!(Critical, "RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}", regs.rsi, regs.rdi, regs.rbp, regs.rsp);
    log!(Critical, "R8 ={:#018x} R9 ={:#018x} R10={:#018x} R11={:#018x}", regs.r8, regs.r9, regs.r10, regs.r11);
    log!(Critical, "R12={:#018x} R13={:#018x} R14={:#018x} R15={:#018x}", regs.r12, regs.r13, regs.r14, regs.r15);
    log!(Critical, "RIP={:#018x} RFLAGS={:#018x}", regs.rip, rflags.bits());
    log!(Critical, "CS={:#06x} SS={:#06x}", cs.0, ss.0);
    log!(
        Critical,
        "CR0={:#018x} CR2={:#018x} CR3={:#018x} CR4={:#018x}",
        cr0.bits(),
        cr2,
        cr3_frame.start_address().as_u64() | cr3_flags.bits(),
        cr4.bits()
    );
    log!(Critical, "EFER={:#018x}", efer.bits());
    log!(Critical, "GS_BASE={:#018x} KGS_BASE={:#018x}", gs_base, kgs_base);
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
