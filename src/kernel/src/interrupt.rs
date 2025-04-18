use crate::driver::acpi;
use crate::driver::acpi::acpi;
use crate::gdt;
use crate::hlt_loop;
use crate::log;
use crate::memory::memory_controller;
use crate::println;
use crate::serial_print;
use apic::LocalApic;
use apic::TimerDivide;
use apic::TimerMode;
use conquer_once::spin::OnceCell;
use io_apic::IoApicManager;
use io_apic::RedirectionTableEntry;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

mod apic;
mod io_apic;

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
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
        idt[InterruptIndex::TimerVector.as_usize()].set_handler_fn(timer);
        idt[InterruptIndex::PITVector.as_usize()].set_handler_fn(pit_timer);
        idt[InterruptIndex::ErrorVector.as_usize()].set_handler_fn(apic_error);
        idt[InterruptIndex::SpuriousInterruptsVector.as_usize()].set_handler_fn(spurious_interrupt);
        idt
    };
}

pub const LOCAL_APIC_OFFSET: u8 = 32;

pub static ID_MAPS: [u8; 256] = [0; 256];
pub static LAPICS: OnceCell<Mutex<LocalApic>> = OnceCell::uninit();
pub static IO_APICS: OnceCell<Mutex<IoApicManager>> = OnceCell::uninit();

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    TimerVector = LOCAL_APIC_OFFSET,
    PITVector,
    ErrorVector,
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

pub fn init() {
    log!(Trace, "Initializing interrupts");

    IDT.load();
    x86_64::instructions::interrupts::enable();

    LAPICS.init_once(|| {
        let mut apic = LocalApic::new(
            InterruptIndex::TimerVector,
            InterruptIndex::ErrorVector,
            InterruptIndex::SpuriousInterruptsVector,
        );
        memory_controller().lock().map_mmio(&mut apic);
        Mutex::new(apic)
    });
    let mut lapic = LAPICS.get().unwrap().lock();
    lapic.enable();
    lapic.start_timer(10_000_00, TimerDivide::Div16, TimerMode::Periodic);
    lapic.enable_timer();
    let apic_id = lapic.id();
    drop(lapic);
    let mut io_apic_manager = IoApicManager::new();
    acpi()
        .lock()
        .io_apics(|addr, gsi_base| io_apic_manager.add_io_apic(addr, gsi_base));
    acpi().lock().interrupt_overrides(|source_override| {
        io_apic_manager.add_source_override(source_override)
    });
    io_apic_manager.redirect_legacy_irqs(
        0,
        RedirectionTableEntry::new(InterruptIndex::PITVector, apic_id),
    );
    IO_APICS.init_once(|| io_apic_manager.into());
}

fn eoi() {
    LAPICS.get().unwrap().lock().eoi();
}

extern "x86-interrupt" fn simd_floating_point_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn x87_floating_point_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn virtualization_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn device_not_available_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn hv_injection_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn invalid_opcode_handler(_stack_frame: InterruptStackFrame) {}

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

extern "x86-interrupt" fn tss_handler(_stack_frame: InterruptStackFrame, _error_code: u64) {}

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

extern "x86-interrupt" fn timer(stack_frame: InterruptStackFrame) {
    eoi();
}

extern "x86-interrupt" fn apic_error(stack_frame: InterruptStackFrame) {
    // TODO : Log the status code
    log!(Error, "APIC ERROR: ");
    eoi();
}

extern "x86-interrupt" fn pit_timer(stack_frame: InterruptStackFrame) {
    eoi();
}

extern "x86-interrupt" fn spurious_interrupt(stack_frame: InterruptStackFrame) {
    log!(Warning, "Spurious interrupt detected");
    eoi();
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
