use core::arch::asm;
use core::fmt;
use core::fmt::LowerHex;
use core::sync::atomic::Ordering;

use crate::gdt::KERNEL_CODE_SEG;
use crate::gdt::KERNEL_DATA_SEG;
use crate::gdt::USER_CODE_SEG;
use crate::gdt::USER_DATA_SEG;
use crate::hlt;
use crate::initialization_context::InitializationContext;
use crate::initialization_context::Stage3;
use crate::initialization_context::Stage4;
use crate::initialize_guard;
use crate::interrupt::apic::ApicId;
use crate::port::Port;
use crate::port::Port8Bit;
use crate::port::PortReadWrite;
use crate::smp::ApInitializationContext;
use crate::smp::CoreId;
use crate::smp::CpuLocalBuilder;
use crate::userland;
use crate::userland::pipeline::dispatch::DispatchAction;
use crate::userland::pipeline::CommonRequestContext;
use crate::userland::pipeline::CommonRequestStackFrame;
use crate::userland::pipeline::RequestReferer;
use crate::PANIC_COUNT;
use alloc::boxed::Box;
use alloc::sync::Arc;
use apic::LocalApic;
use apic::LocalApicArguments;
use idt::Idt;
use idt::InterruptStackFrame;
use idt::PageFaultErrorCode;
use io_apic::IoApicManager;
use io_apic::RedirectionTableEntry;
use kernel_proc::def_local;
use kernel_proc::local_builder;
use kernel_proc::{fill_idt, generate_interrupt_handlers};
use pager::address::VirtAddr;
use pager::gdt::DOUBLE_FAULT_IST_INDEX;
use pager::gdt::GENERAL_STACK_INDEX;
use pager::registers::Cr2;
use pager::registers::RFlags;
use sentinel::log;
use spin::Mutex;

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
    CheckIPP = 0x95,
    SpuriousInterruptsVector = 0xFF,
}

impl TryFrom<u8> for InterruptIndex {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            v if v == Self::TimerVector as u8 => Ok(Self::TimerVector),
            v if v == Self::PITVector as u8 => Ok(Self::PITVector),
            v if v == Self::ErrorVector as u8 => Ok(Self::ErrorVector),
            v if v == Self::DriverCall as u8 => Ok(Self::DriverCall),
            v if v == Self::CheckFutex as u8 => Ok(Self::CheckFutex),
            v if v == Self::CheckIPP as u8 => Ok(Self::CheckIPP),
            v if v == Self::SpuriousInterruptsVector as u8 => Ok(Self::SpuriousInterruptsVector),
            v => Err(v),
        }
    }
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

    unsafe {
        idt.general_protection
            .set_handler_fn(general_protection_fault_handler)
            .set_stack_index(GENERAL_STACK_INDEX);
        idt.page_fault
            .set_handler_fn(page_fault_handler)
            .set_stack_index(GENERAL_STACK_INDEX);
        idt.invalid_opcode
            .set_handler_addr(VirtAddr::new(invalid_opcode as *const () as u64))
            .set_stack_index(GENERAL_STACK_INDEX);
        idt.break_point.set_handler_fn(break_point);
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(DOUBLE_FAULT_IST_INDEX);
    }
    fill_idt!();
    idt
}

def_local!(static IDT: &'static crate::interrupt::idt::Idt);
def_local!(pub static LAPIC: crate::interrupt::apic::LocalApic);
def_local!(pub static CORE_ID: CoreId);
def_local!(pub static APIC_ID: ApicId);
def_local!(pub static LAST_INTERRUPT_NO: u8);
def_local!(pub static IS_IN_ISR: bool);
def_local!(pub static TPMS: usize);
def_local!(pub static IO_APIC: Arc<Mutex<IoApicManager>>);

pub fn init(mut ctx: InitializationContext<Stage3>) -> InitializationContext<Stage4> {
    initialize_guard!();

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

    let lapic = move |cpu: &mut CpuLocalBuilder, ctx: &ApInitializationContext, id| {
        log!(Info, "Initializing interrupts for CPU: {id}");
        let idt = create_idt();
        idt.load();
        let mut lapic = lapic.clone();
        lapic.enable();
        lapic.disable_timer();

        let apic_id = lapic.id();
        let core_id = Into::<CoreId>::into(lapic.id());
        local_builder!(
            cpu,
            IDT(idt),
            LAPIC(lapic),
            APIC_ID(apic_id),
            CORE_ID(core_id),
            LAST_INTERRUPT_NO(0),
            IS_IN_ISR(false),
            TPMS(1000),
            IO_APIC(Arc::clone(&ctx.io_apic)),
        );
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

    let lapic_calibration = |id| {
        log!(Trace, "Calibrating APIC for cpu: {id}");
        IO_APIC.lock().redirect_legacy_irqs(
            0,
            RedirectionTableEntry::new(InterruptIndex::PITVector, *APIC_ID),
        );
        LAPIC.inner_mut().calibrate();
    };

    ctx.local_initializer(|initializer| {
        initializer.context_transformer(|builder, ctx| {
            builder.io_apic(Arc::new(ctx.context.take_io_apic_manager().unwrap().into()));
        });
        initializer.register(lapic);

        initializer.after_bsp(|| {
            IO_APIC.lock().redirect_legacy_irqs(
                0,
                RedirectionTableEntry::new(InterruptIndex::PITVector, *APIC_ID),
            );
        });

        initializer.register_after(lapic_calibration);
    });
    ctx.next(io_apic_manager)
}

impl From<&ExtendedInterruptStackFrame> for CommonRequestStackFrame {
    fn from(value: &ExtendedInterruptStackFrame) -> Self {
        Self {
            r15: value.r15,
            r14: value.r14,
            r13: value.r13,
            r12: value.r12,
            r11: value.r11,
            r10: value.r10,
            r9: value.r9,
            r8: value.r8,
            rsi: value.rsi,
            rdi: value.rdi,
            rbp: value.rbp,
            rdx: value.rdx,
            rcx: value.rcx,
            rbx: value.rbx,
            rax: value.rax,
            instruction_pointer: value.instruction_pointer,
            cpu_flags: value.cpu_flags,
            stack_pointer: value.stack_pointer,
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct ExtendedInterruptStackFrame {
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
    pub cpu_flags: RFlags,
    pub stack_pointer: VirtAddr,
    pub stack_segment: u64,
}

impl ExtendedInterruptStackFrame {
    fn replace_with(&mut self, c_stack: &CommonRequestStackFrame) {
        self.r15 = c_stack.r15;
        self.r14 = c_stack.r14;
        self.r13 = c_stack.r13;
        self.r12 = c_stack.r12;
        self.r11 = c_stack.r11;
        self.r10 = c_stack.r10;
        self.r9 = c_stack.r9;
        self.r8 = c_stack.r8;
        self.rsi = c_stack.rsi;
        self.rdi = c_stack.rdi;
        self.rbp = c_stack.rbp;
        self.rdx = c_stack.rdx;
        self.rcx = c_stack.rcx;
        self.rbx = c_stack.rbx;
        self.rax = c_stack.rax;
        self.instruction_pointer = c_stack.instruction_pointer;
        self.cpu_flags = c_stack.cpu_flags;
        self.stack_pointer = c_stack.stack_pointer;
    }
}

impl fmt::Debug for ExtendedInterruptStackFrame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        struct Hex<T: LowerHex>(T);
        impl<T: LowerHex> fmt::Debug for Hex<T> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{:#x}", self.0)
            }
        }

        let mut s = f.debug_struct("InterruptStackFrame");
        s.field("r15", &Hex(self.r15));
        s.field("r14", &Hex(self.r14));
        s.field("r13", &Hex(self.r13));
        s.field("r12", &Hex(self.r12));
        s.field("r11", &Hex(self.r11));
        s.field("r10", &Hex(self.r10));
        s.field("r9", &Hex(self.r9));
        s.field("r8", &Hex(self.r8));
        s.field("rsi", &Hex(self.rsi));
        s.field("rdi", &Hex(self.rdi));
        s.field("rbp", &Hex(self.rbp));
        s.field("rdx", &Hex(self.rdx));
        s.field("rcx", &Hex(self.rcx));
        s.field("rbx", &Hex(self.rbx));
        s.field("rax", &Hex(self.rax));
        s.field("instruction_pointer", &Hex(self.instruction_pointer));
        s.field("code_segment", &self.code_segment);
        s.field("cpu_flags", &Hex(self.cpu_flags));
        s.field("stack_pointer", &Hex(self.stack_pointer));
        s.field("stack_segment", &self.stack_segment);
        s.finish()
    }
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
    let was_enable = RFlags::read().contains(RFlags::InterruptEnable);
    if was_enable {
        disable();
    }

    let ret = f();

    if was_enable {
        enable();
    }
    ret
}

fn hlt_loop() {
    loop {
        hlt();
    }
}

#[unsafe(no_mangle)]
extern "C" fn external_interrupt_handler(stack_frame: &mut ExtendedInterruptStackFrame, idx: u8) {
    if PANIC_COUNT.load(Ordering::SeqCst) > 0 {
        eoi(idx);
        disable();
        return;
    }

    *LAST_INTERRUPT_NO.inner_mut() = idx;
    *IS_IN_ISR.inner_mut() = true;

    let idx = match InterruptIndex::try_from(idx) {
        Ok(index) => index,
        Err(..) => {
            eoi(idx);
            *IS_IN_ISR.inner_mut() = false;
            return;
        }
    };

    let mut c_stack_frame = CommonRequestStackFrame::from(&*stack_frame);

    userland::pipeline::handle_request(
        CommonRequestContext::new(&mut c_stack_frame, RequestReferer::HardwareInterrupt(idx)),
        |CommonRequestContext {
             stack_frame: c_stack_frame,
             ..
         },
         dispatcher| {
            dispatcher.dispatch(|action| match action {
                DispatchAction::HltLoop => {
                    c_stack_frame.instruction_pointer = VirtAddr::new(hlt_loop as *const () as u64);

                    stack_frame.code_segment = KERNEL_CODE_SEG.0.into();
                    stack_frame.stack_segment = KERNEL_DATA_SEG.0.into();
                }
                DispatchAction::ReplaceState(state) => {
                    c_stack_frame.replace_with(state);

                    stack_frame.code_segment = USER_CODE_SEG.0.into();
                    stack_frame.stack_segment = USER_DATA_SEG.0.into();
                }
            })
        },
    );

    stack_frame.replace_with(&c_stack_frame);

    eoi(idx as u8);
    *IS_IN_ISR.inner_mut() = false;
}

generate_interrupt_handlers!();

macro_rules! handler {
    ($vis:vis fn $fn_name: ident($stack_frame_name: ident : $stack_frame_ty: ty) { $($body:tt)* }) => {
        paste::paste! {
            #[unsafe(no_mangle)]
            #[unsafe(naked)]
            $vis extern "C" fn $fn_name() {
                #[unsafe(no_mangle)]
                fn [<handler_ $fn_name>]($stack_frame_name: $stack_frame_ty) {
                    $($body)*
                }

                core::arch::naked_asm!(
                    "push rax",
                    "push rbx",
                    "push rcx",
                    "push rdx",
                    "push rbp",
                    "push rdi",
                    "push rsi",
                    "push r8",
                    "push r9",
                    "push r10",
                    "push r11",
                    "push r12",
                    "push r13",
                    "push r14",
                    "push r15",
                    "mov rdi, rsp",
                    concat!("call ", stringify!([<handler_ $fn_name>])),
                    "pop r15",
                    "pop r14",
                    "pop r13",
                    "pop r12",
                    "pop r11",
                    "pop r10",
                    "pop r9",
                    "pop r8",
                    "pop rsi",
                    "pop rdi",
                    "pop rbp",
                    "pop rdx",
                    "pop rcx",
                    "pop rbx",
                    "pop rax",
                    // Return from interrupt
                    "iretq",
                );
            }
        }
    };
}

fn eoi(idx: u8) {
    if idx != InterruptIndex::DriverCall.as_u8() {
        LAPIC.inner_mut().eoi();
    }
}

handler!(
    fn invalid_opcode(stack_frame: ExtendedInterruptStackFrame) {
        panic!("EXCEPTION: INVALID OPCODE\n{:#?}", stack_frame);
    }
);

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT\n{:#?}, ERROR_CODE: {}",
        stack_frame, error_code
    );
}

extern "x86-interrupt" fn break_point(_stack_frame: InterruptStackFrame) {}

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
