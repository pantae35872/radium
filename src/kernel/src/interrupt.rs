use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

use crate::driver::acpi::acpi;
use crate::gdt;
use crate::hlt_loop;
use crate::log;
use crate::logger::LOGGER;
use crate::memory::memory_controller;
use crate::println;
use crate::serial_print;
use crate::serial_println;
use crate::smp::cpu_local;
use crate::smp::local_initializer;
use crate::smp::CpuLocalBuilder;
use crate::utils::port::Port8Bit;
use alloc::boxed::Box;
use apic::LocalApic;
use apic::TimerDivide;
use apic::TimerMode;
use conquer_once::spin::OnceCell;
use io_apic::IoApicManager;
use io_apic::RedirectionTableEntry;
use kernel_proc::{fill_idt, generate_interrupt_handlers};
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use x86_64::VirtAddr;

pub mod apic;
pub mod io_apic;

pub const LOCAL_APIC_OFFSET: u8 = 32;

pub static TIMER_COUNT: AtomicUsize = AtomicUsize::new(0);
static IO_APICS: OnceCell<IoApicManager> = OnceCell::uninit();

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

fn create_idt() -> &'static InterruptDescriptorTable {
    let idt = Box::leak(InterruptDescriptorTable::new().into());
    idt.breakpoint.set_handler_fn(breakpoint_handle);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.overflow.set_handler_fn(overflow_handler);
    idt.divide_error.set_handler_fn(divide_handler);
    idt.debug.set_handler_fn(debug_handler);
    idt.invalid_tss.set_handler_fn(tss_handler);
    idt.machine_check.set_handler_fn(machine_check_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    idt.hv_injection_exception
        .set_handler_fn(hv_injection_handler);
    idt.device_not_available
        .set_handler_fn(device_not_available_handler);
    idt.vmm_communication_exception
        .set_handler_fn(vmm_communication_exception_handler);
    idt.virtualization.set_handler_fn(virtualization_handler);
    idt.security_exception
        .set_handler_fn(security_exception_handler);
    idt.alignment_check.set_handler_fn(alignment_check_handler);
    idt.x87_floating_point
        .set_handler_fn(x87_floating_point_handler);
    idt.segment_not_present
        .set_handler_fn(segment_not_present_handler);
    idt.general_protection_fault
        .set_handler_fn(general_protection_fault_handler);
    idt.cp_protection_exception
        .set_handler_fn(cp_protection_exception_handler);
    idt.stack_segment_fault
        .set_handler_fn(stack_segment_fault_handler);
    idt.simd_floating_point
        .set_handler_fn(simd_floating_point_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    }
    fill_idt!();
    idt
}

pub fn init() {
    // FIXME: The apic version doesn't work correctly in the aps because the memory are being
    // mapped multiple times
    local_initializer()
        .lock()
        .register(|cpu: &mut CpuLocalBuilder, id| {
            log!(Info, "Initializing interrupts for CPU: {id}");
            disable_pic();
            let idt = create_idt();
            idt.load();
            cpu.idt(idt);
            x86_64::instructions::interrupts::enable();
            let mut lapic = LocalApic::new(
                InterruptIndex::TimerVector,
                InterruptIndex::ErrorVector,
                InterruptIndex::SpuriousInterruptsVector,
            );
            memory_controller().lock().map_mmio(&mut lapic, true);
            lapic.enable();
            lapic.start_timer(1_000_000, TimerDivide::Div128, TimerMode::Periodic);
            lapic.enable_timer();
            cpu.lapic(lapic);
        });
    local_initializer().lock().after_bsp(|bsp| {
        let mut io_apic_manager = IoApicManager::new();
        acpi()
            .lock()
            .io_apics(|addr, gsi_base| io_apic_manager.add_io_apic(addr, gsi_base));
        acpi().lock().interrupt_overrides(|source_override| {
            io_apic_manager.add_source_override(source_override)
        });
        io_apic_manager.redirect_legacy_irqs(
            0,
            RedirectionTableEntry::new(InterruptIndex::PITVector, bsp.apic_id()),
        );
        IO_APICS.init_once(|| io_apic_manager.into());
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
    pub cpu_flags: u64,
    pub stack_pointer: VirtAddr,
    pub stack_segment: u64,
}

#[no_mangle]
extern "C" fn external_interrupt_handler(stack_frame: &mut FullInterruptStackFrame, idx: u8) {
    match idx {
        idx if idx == InterruptIndex::TimerVector.as_u8() => {
            serial_println!("APIC timer on cpu: {}", cpu_local().cpu_id());
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

extern "x86-interrupt" fn simd_floating_point_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn x87_floating_point_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn virtualization_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn device_not_available_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn hv_injection_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn invalid_opcode_handler(_stack_frame: InterruptStackFrame) {
    panic!("Inavlid opcode");
}

extern "x86-interrupt" fn machine_check_handler(_stack_frame: InterruptStackFrame) -> ! {
    hlt_loop();
}

extern "x86-interrupt" fn stack_segment_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
}

extern "x86-interrupt" fn cp_protection_exception_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
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

extern "x86-interrupt" fn segment_not_present_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
}

extern "x86-interrupt" fn alignment_check_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
}

extern "x86-interrupt" fn security_exception_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
}

extern "x86-interrupt" fn vmm_communication_exception_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
}

extern "x86-interrupt" fn tss_handler(_stack_frame: InterruptStackFrame, _error_code: u64) {
    panic!("INVALID TSS");
}

extern "x86-interrupt" fn debug_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn divide_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: DIVISION\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn overflow_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: OVERFLOW\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn breakpoint_handle(_stack_frame: InterruptStackFrame) {
    println!("BreakPoint");
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

    log!(Critical, "EXCEPTION: PAGE FAULT");
    log!(Critical, "Accessed Address: {:?}", Cr2::read());
    log!(Critical, "Error Code: {:?}", error_code);
    log!(Critical, "{:#?}", stack_frame);
    panic!("PAGE FAULT");
}
