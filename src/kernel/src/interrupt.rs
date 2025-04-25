use core::arch::asm;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

use crate::gdt;
use crate::initialization_context::InitializationContext;
use crate::initialization_context::Phase3;
use crate::log;
use crate::serial_println;
use crate::smp::cpu_local;
use crate::smp::CpuLocalBuilder;
use crate::utils::port::Port8Bit;
use alloc::boxed::Box;
use apic::LocalApic;
use apic::LocalApicArguments;
use apic::TimerDivide;
use apic::TimerMode;
use idt::Idt;
use idt::InterruptStackFrame;
use idt::PageFaultErrorCode;
use io_apic::IoApicManager;
use io_apic::RedirectionTableEntry;
use kernel_proc::{fill_idt, generate_interrupt_handlers};
use pager::address::VirtAddr;
use pager::registers::Cr2;
use pager::registers::RFlags;
use pager::registers::RFlagsFlags;

pub mod apic;
pub mod idt;
pub mod io_apic;

pub const LOCAL_APIC_OFFSET: u8 = 32;

pub static TIMER_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    TimerVector = LOCAL_APIC_OFFSET,
    PITVector,
    ErrorVector,
    DriverCall = 0x90,
    SpuriousInterruptsVector = 0xFF,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}

fn disable_pic() {
    unsafe {
        Port8Bit::new(0x21).write(0xff);
        Port8Bit::new(0xA1).write(0xff);
    }
}

fn create_idt() -> &'static Idt {
    let idt = Box::leak(Idt::new().into());
    idt.general_protection
        .set_handler_fn(general_protection_fault_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    }
    fill_idt!();
    idt
}

pub fn init(ctx: &mut InitializationContext<Phase3>) {
    let lapic = ctx
        .mmio_device::<LocalApic, _>(
            LocalApicArguments {
                timer_vector: InterruptIndex::TimerVector,
                error_vector: InterruptIndex::ErrorVector,
                spurious_vector: InterruptIndex::SpuriousInterruptsVector,
            },
            None,
        )
        .unwrap();
    let lapic = move |cpu: &mut CpuLocalBuilder, _ctx: &mut InitializationContext<Phase3>, id| {
        let mut lapic = lapic.clone();
        log!(Info, "Initializing interrupts for CPU: {id}");
        disable_pic();
        let idt = create_idt();
        idt.load();
        cpu.idt(idt);
        enable();

        lapic.enable();
        lapic.start_timer(1_000_000, TimerDivide::Div128, TimerMode::Periodic);
        lapic.enable_timer();
        cpu.lapic(lapic);
    };
    ctx.local_initializer(|e| e.register(lapic));
    let mut io_apic_manager = IoApicManager::new();
    let io_apics = ctx.context().io_apics().clone();
    io_apics
        .iter()
        .for_each(|(addr, gsi_base)| io_apic_manager.add_io_apic(addr.clone(), *gsi_base, ctx));
    ctx.context()
        .interrupt_source_overrides()
        .iter()
        .for_each(|source_override| io_apic_manager.add_source_override(source_override));
    ctx.local_initializer(|initializer| {
        initializer.after_bsp(move |bsp| {
            io_apic_manager.redirect_legacy_irqs(
                0,
                RedirectionTableEntry::new(InterruptIndex::PITVector, bsp.apic_id()),
            );
        });
    });
}

#[derive(Debug)]
#[repr(C)]
struct FullInterruptStackFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub instruction_pointer: VirtAddr,
    pub code_segment: u64,
    pub cpu_flags: RFlagsFlags,
    pub stack_pointer: VirtAddr,
    pub stack_segment: u64,
}

#[inline(always)]
pub fn disable() {
    // SAFETY: Enabling and Disabling interrupt is considered safe in kernel context
    unsafe { asm!("cli", options(nomem, nostack)) }
}

#[inline(always)]
pub fn enable() {
    // SAFETY: Enabling and Disabling interrupt is considered safe in kernel context
    unsafe { asm!("sti", options(nomem, nostack)) }
}

#[inline(always)]
pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let was_enable = RFlags::read().contains(RFlagsFlags::InterruptEnable);
    if was_enable {
        disable();
    }

    let ret = f();

    if was_enable {
        enable();
    }
    ret
}

#[unsafe(no_mangle)]
extern "C" fn external_interrupt_handler(stack_frame: &mut FullInterruptStackFrame, idx: u8) {
    match idx {
        idx if idx == InterruptIndex::TimerVector.as_u8() => {
            serial_println!("APIC timer on cpu: {}", cpu_local().cpu_id());
            serial_println!("Flags: {:?}", stack_frame.cpu_flags);
        }
        idx if idx == InterruptIndex::PITVector.as_u8() => {
            TIMER_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        idx if idx == InterruptIndex::ErrorVector.as_u8() => {
            log!(Error, "Apic configuration error");
        }
        idx if idx == InterruptIndex::SpuriousInterruptsVector.as_u8() => {
            log!(Warning, "Spurious Interrupt Detected");
        }
        idx if idx == InterruptIndex::DriverCall.as_u8() => todo!(),
        idx => {
            log!(Error, "Unhandled external interrupts {}", idx);
        }
    }

    if idx != InterruptIndex::DriverCall.as_u8() {
        eoi();
    }
}

generate_interrupt_handlers!();

fn eoi() {
    cpu_local().lapic().eoi();
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT\n{:#?}, ERROR_CODE: {}",
        stack_frame, error_code
    );
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    panic!(
        "EXCEPTION: DOUBLE FAULT\n{:#?}, ERROR_CODE: {}",
        stack_frame, error_code
    );
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    log!(Critical, "EXCEPTION: PAGE FAULT");
    log!(Critical, "Accessed Address: {:x?}", Cr2::read());
    log!(Critical, "Error Code: {:?}", error_code);
    log!(Critical, "{:#?}", stack_frame);
    panic!("PAGE FAULT");
}
