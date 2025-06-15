use core::arch::asm;
use core::sync::atomic::Ordering;

use crate::initialization_context::End;
use crate::initialization_context::InitializationContext;
use crate::initialization_context::Stage3;
use crate::port::Port;
use crate::port::Port8Bit;
use crate::port::PortReadWrite;
use crate::scheduler::Dispatcher;
use crate::scheduler::DRIVCALL_EXIT;
use crate::scheduler::DRIVCALL_FUTEX_WAIT;
use crate::scheduler::DRIVCALL_FUTEX_WAKE;
use crate::scheduler::DRIVCALL_SLEEP;
use crate::scheduler::DRIVCALL_SPAWN;
use crate::scheduler::DRIVCALL_VSYS_REG;
use crate::scheduler::DRIVCALL_VSYS_REQ;
use crate::scheduler::DRIVCALL_VSYS_RET;
use crate::scheduler::DRIVCALL_VSYS_WAIT;
use crate::serial_println;
use crate::smp::cpu_local;
use crate::smp::CpuLocalBuilder;
use crate::PANIC_COUNT;
use alloc::boxed::Box;
use apic::LocalApic;
use apic::LocalApicArguments;
use idt::Idt;
use idt::InterruptStackFrame;
use idt::PageFaultErrorCode;
use io_apic::IoApicManager;
use io_apic::RedirectionTableEntry;
use kernel_proc::{fill_idt, generate_interrupt_handlers};
use pager::address::VirtAddr;
use pager::gdt::DOUBLE_FAULT_IST_INDEX;
use pager::gdt::GENERAL_STACK_INDEX;
use pager::registers::Cr2;
use pager::registers::RFlags;
use pager::registers::RFlagsFlags;
use sentinel::log;

pub mod apic;
pub mod idt;
pub mod io_apic;

pub const LOCAL_APIC_OFFSET: u8 = 32;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    TimerVector = LOCAL_APIC_OFFSET,
    PITVector,
    ErrorVector,
    DriverCall = 0x90,
    CheckFutex = 0x92,
    SpuriousInterruptsVector = 0xFF,
}

impl InterruptIndex {
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[inline]
    pub const fn as_usize(self) -> usize {
        self.as_u8() as usize
    }
}

fn disable_pic(ctx: &mut InitializationContext<Stage3>) {
    let mut pic_1_data: Port<Port8Bit, PortReadWrite> =
        ctx.alloc_port(0x21).expect("PIC Port is already taken");
    let mut pic_2_data: Port<Port8Bit, PortReadWrite> =
        ctx.alloc_port(0xA1).expect("PIC Port is already taken");
    unsafe {
        pic_1_data.write(0xff);
        pic_2_data.write(0xff);
    }
}

fn create_idt() -> &'static Idt {
    let idt = Box::leak(Idt::new().into());
    idt.general_protection
        .set_handler_fn(general_protection_fault_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(DOUBLE_FAULT_IST_INDEX);
    }
    fill_idt!();
    idt
}

pub fn init(mut ctx: InitializationContext<Stage3>) -> InitializationContext<End> {
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
    disable_pic(&mut ctx);

    let lapic = move |cpu: &mut CpuLocalBuilder, _ctx: &mut InitializationContext<End>, id| {
        log!(Info, "Initializing interrupts for CPU: {id}");
        let idt = create_idt();
        idt.load();
        cpu.idt(idt);
        let mut lapic = lapic.clone();
        lapic.enable();
        lapic.disable_timer();

        cpu.lapic(lapic);
    };

    let mut io_apic_manager = IoApicManager::new();
    let io_apics = ctx.context().io_apics().clone();
    io_apics.iter().for_each(|(addr, gsi_base)| {
        io_apic_manager.add_io_apic(addr.clone(), *gsi_base, &mut ctx)
    });
    ctx.context()
        .interrupt_source_overrides()
        .iter()
        .for_each(|source_override| io_apic_manager.add_source_override(source_override));

    let lapic_calibration = |ctx: &mut InitializationContext<End>, id| {
        log!(Trace, "Calibrating APIC for cpu: {id}");
        ctx.redirect_legacy_irqs(
            0,
            RedirectionTableEntry::new(InterruptIndex::PITVector, cpu_local().apic_id()),
        );
        cpu_local().lapic().calibrate();
    };

    ctx.local_initializer(|initializer| {
        initializer.register(lapic);

        initializer.after_bsp(|ctx| {
            ctx.context.io_apic_manager.redirect_legacy_irqs(
                0,
                RedirectionTableEntry::new(InterruptIndex::PITVector, cpu_local().apic_id()),
            );
        });

        initializer.register_after(lapic_calibration);
    });
    ctx.next(io_apic_manager)
}

#[derive(Debug)]
#[repr(C)]
pub struct FullInterruptStackFrame {
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
    if PANIC_COUNT.load(Ordering::SeqCst) > 0 {
        eoi(idx);
        disable();
        return;
    }

    cpu_local().last_interrupt_no = idx;
    cpu_local().is_in_isr = true;

    let current_thread = Dispatcher::save(stack_frame);
    let mut is_scheduleable_interrupt = false;

    match idx {
        idx if idx == InterruptIndex::TimerVector.as_u8() => {
            cpu_local().local_scheduler().prepare_timer();
            cpu_local().local_scheduler().check_migrate();
            cpu_local().local_scheduler().check_return();
            cpu_local().local_scheduler().check_vsys_request();
            cpu_local().local_scheduler().push_thread(current_thread);
            is_scheduleable_interrupt = true;
        }
        idx if idx == InterruptIndex::PITVector.as_u8() => {}
        idx if idx == InterruptIndex::ErrorVector.as_u8() => {
            log!(Error, "Apic configuration error");
        }
        idx if idx == InterruptIndex::SpuriousInterruptsVector.as_u8() => {
            log!(Warning, "Spurious Interrupt Detected");
        }
        // FIXME: use simulated interrupts instead of software interrupts
        idx if idx == InterruptIndex::DriverCall.as_u8() => match stack_frame.rdi {
            DRIVCALL_SLEEP => {
                cpu_local()
                    .local_scheduler()
                    .sleep_thread(current_thread, stack_frame.rax as usize);
                is_scheduleable_interrupt = true;
            }
            DRIVCALL_SPAWN => todo!("Implement Spawn drivcall"),
            DRIVCALL_FUTEX_WAIT => {
                cpu_local().local_scheduler().futex_wait(
                    VirtAddr::new(stack_frame.rax),
                    current_thread,
                    stack_frame.rcx as usize,
                );

                is_scheduleable_interrupt = true;
            }
            DRIVCALL_FUTEX_WAKE => {
                cpu_local()
                    .local_scheduler()
                    .futex_wake(VirtAddr::new(stack_frame.rax));
            }
            DRIVCALL_EXIT => {
                cpu_local().local_scheduler().exit_thread(current_thread);
                is_scheduleable_interrupt = true;
            }
            DRIVCALL_VSYS_REG => {
                // TODO: Check if the request vsys is out of range
                cpu_local()
                    .local_scheduler()
                    .vsys_reg(stack_frame.rax as usize, current_thread.global_id());
            }
            DRIVCALL_VSYS_WAIT => {
                cpu_local()
                    .local_scheduler()
                    .vsys_wait(stack_frame.rax as usize, current_thread);
                is_scheduleable_interrupt = true;
            }
            DRIVCALL_VSYS_REQ => {
                cpu_local()
                    .local_scheduler()
                    .vsys_req(stack_frame.rax as usize, current_thread);
                is_scheduleable_interrupt = true;
            }
            DRIVCALL_VSYS_RET => {
                cpu_local()
                    .local_scheduler()
                    .vsys_return_thread(current_thread);
            }
            number => log!(Error, "Unknown Driver call called, {number}"),
        },
        idx if idx == InterruptIndex::CheckFutex.as_u8() => {
            cpu_local().local_scheduler().check_futex();
        }
        idx => {
            log!(Error, "Unhandled external interrupts {}", idx);
            return;
        }
    }

    if is_scheduleable_interrupt {
        if let Some(sched_thread) = cpu_local().local_scheduler().schedule() {
            Dispatcher::dispatch(stack_frame, sched_thread);
        }
    }

    eoi(idx);
    cpu_local().is_in_isr = false;
}

generate_interrupt_handlers!();

fn eoi(idx: u8) {
    if idx != InterruptIndex::DriverCall.as_u8() {
        cpu_local().lapic().eoi();
    }
}

extern "x86-interrupt" fn invalid_opcode(stack_frame: InterruptStackFrame) {
    panic!("EXCEPTION: INVALID OPCODE\n{:#?}", stack_frame);
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
