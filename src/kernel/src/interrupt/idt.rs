use core::{
    fmt,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use alloc::boxed::Box;
use bit_field::BitField;
use pager::{
    address::VirtAddr,
    registers::{lidt, Cr2, DescriptorTablePointer, SegmentSelector, CS},
};
use x86_64::structures::idt::InterruptDescriptorTable;

use crate::{hlt_loop, log};

#[derive(Clone, Copy)]
pub struct GateInterrupt;

#[derive(Clone, Copy)]
pub struct GateTrap;

pub trait GateType {
    fn as_u8() -> u8;
}

impl GateType for GateInterrupt {
    fn as_u8() -> u8 {
        0b1110
    }
}

impl GateType for GateTrap {
    fn as_u8() -> u8 {
        0b1111
    }
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct GateOptions<T: GateType>(u16, PhantomData<T>);

macro_rules! handler_fn_impl {
    ($($handler:ident => $default_name: ident (
            $($name:ident : $type:ty),*
    ) $(-> $ret:ty)? $default: block)*) => {$(
        pub type $handler = extern "x86-interrupt" fn($($name: $type),*) $(-> $ret)?;

        extern "x86-interrupt" fn $default_name($($name: $type),*) $(-> $ret)? $default

        impl InterruptHandler for $handler {
            fn to_virt_addr(self) -> VirtAddr {
                VirtAddr::new(self as u64)
            }
        }

        impl<T: GateType> Default for Gate<$handler, T> {
            fn default() -> Self {
                let mut entry = Self::new();
                entry.set_handler_fn($default_name);
                entry
            }
        }
    )*};
}

pub trait InterruptHandler {
    fn to_virt_addr(self) -> VirtAddr;
}

handler_fn_impl! {
    NormalHandler => missing_gate(stack_frame: InterruptStackFrame) {
        log!(Critical, "UNHANDLED CPU EXCEPTIONS");
        log!(Critical, "{:#?}", stack_frame);
        panic!("Unhandled Exceptions");
    }
    HandlerWithErrorCode => missing_gate_with_error_code(stack_frame: InterruptStackFrame, error_code: u64) {
        log!(Critical, "UNHANDLED CPU EXCEPTIONS");
        log!(Critical, "Error Code: {:?}", error_code);
        log!(Critical, "{:#?}", stack_frame);
        panic!("Unhandled Exceptions");
    }
    NoReturnHandler => missing_gate_no_return(stack_frame: InterruptStackFrame) -> ! {
        log!(Critical, "UNHANDLED CPU EXCEPTIONS");
        log!(Critical, "{:#?}", stack_frame);
        panic!("UNHANDLED CPU EXCEPTIONS");
    }
    NoReturnHandlerWithErrorCode => missing_gate_with_error_code_no_return(stack_frame: InterruptStackFrame, error_code: u64) -> ! {
        log!(Critical, "UNHANDLED CPU EXCEPTIONS");
        log!(Critical, "Error Code: {:?}", error_code);
        log!(Critical, "{:#?}", stack_frame);
        panic!("UNHANDLED CPU EXCEPTIONS");
    }
    PageFaultHandler => page_fault_handler(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode) {
        log!(Critical, "UNHANDLED PAGE FAULT");
        log!(Critical, "EXCEPTION: PAGE FAULT");
        log!(Critical, "Accessed Address: {:?}", Cr2::read());
        log!(
            Critical,
            "ERROR CODE: {:?}",
            error_code
        );
        log!(Critical, "{:#?}", stack_frame);
        panic!("UNHANDLED PAGE FAULT");
    }
}

struct ReservedGate;

impl InterruptHandler for ReservedGate {
    fn to_virt_addr(self) -> VirtAddr {
        panic!("Trying to use reserved gate");
    }
}

impl<T: GateType> Default for Gate<ReservedGate, T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct InterruptStackFrame {
    pub instruction_pointer: VirtAddr,
    pub code_segment: u64,
    pub cpu_flags: u64,
    pub stack_pointer: VirtAddr,
    pub stack_segment: u64,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Gate<F, T: GateType> {
    addr_low: u16,
    segment_selector: SegmentSelector,
    options: GateOptions<T>,
    addr_mid: u16,
    addr_high: u32,
    _reserved: u32,
    _gate_type: PhantomData<F>,
}

#[repr(C)]
#[repr(align(16))]
pub struct Idt {
    // Vector nr 0
    pub divide_error: Gate<NormalHandler, GateTrap>,
    // Vector nr 1
    pub debug_excepton: Gate<NormalHandler, GateTrap>,
    // Vector nr 2
    pub nmi_external: Gate<NormalHandler, GateInterrupt>,
    // Vector nr 3
    pub break_point: Gate<NormalHandler, GateTrap>,
    // Vector nr 4
    pub overflow: Gate<NormalHandler, GateTrap>,
    // Vector nr 5
    pub bound_range_exceeded: Gate<NormalHandler, GateTrap>,
    // Vector nr 6
    pub invalid_opcode: Gate<NormalHandler, GateTrap>,
    // Vector nr 7
    pub device_not_available: Gate<NormalHandler, GateTrap>,
    // Vector nr 8
    pub double_fault: Gate<NoReturnHandlerWithErrorCode, GateTrap>,
    // Vector nr 9
    _coprocessor_segment_overrun: Gate<ReservedGate, GateTrap>, // Reserved
    // Vector nr 10
    pub invalid_tss: Gate<HandlerWithErrorCode, GateTrap>,
    // Vector nr 11
    pub segment_not_present: Gate<HandlerWithErrorCode, GateTrap>,
    // Vector nr 12
    pub stack_segment_fault: Gate<HandlerWithErrorCode, GateTrap>,
    // Vector nr 13
    pub general_protection: Gate<HandlerWithErrorCode, GateTrap>,
    // Vector nr 14
    pub page_fault: Gate<PageFaultHandler, GateTrap>,
    // Vector nr 15
    _intel_reserved: Gate<ReservedGate, GateTrap>,
    // Vector nr 16
    pub math_fault: Gate<NormalHandler, GateTrap>,
    // Vector nr 17
    pub alignment_check: Gate<HandlerWithErrorCode, GateTrap>,
    // Vector nr 18
    pub machine_check: Gate<NoReturnHandler, GateTrap>,
    // Vector nr 19
    pub simd_exception: Gate<NormalHandler, GateTrap>,
    // Vector nr 20
    pub virtualization_exception: Gate<NormalHandler, GateTrap>,
    // Vector nr 21
    pub control_protection_exceptoin: Gate<HandlerWithErrorCode, GateTrap>,
    // Vector nr 21-31 inclusive
    _reserved: [Gate<NormalHandler, GateTrap>; 10],
    external_interrupts: [Gate<NormalHandler, GateInterrupt>; 224],
}

impl fmt::Debug for InterruptStackFrame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        struct Hex(u64);
        impl fmt::Debug for Hex {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{:#x}", self.0)
            }
        }

        let mut s = f.debug_struct("InterruptStackFrame");
        s.field("instruction_pointer", &self.instruction_pointer);
        s.field("code_segment", &self.code_segment);
        s.field("cpu_flags", &Hex(self.cpu_flags));
        s.field("stack_pointer", &self.stack_pointer);
        s.field("stack_segment", &self.stack_segment);
        s.finish()
    }
}

impl<T: GateType> GateOptions<T> {
    // Create a new trap gate options with present bits set
    pub fn new() -> Self {
        Self((T::as_u8() as u16) << 8, PhantomData)
    }

    pub fn set_present(&mut self, present: bool) -> &mut Self {
        self.0.set_bit(15, present);
        self
    }

    pub unsafe fn set_stack_index(&mut self, index: u16) -> &mut Self {
        self.0.set_bits(0..3, index + 1);
        self
    }
}

impl<F: InterruptHandler, T: GateType> Gate<F, T> {
    fn new() -> Self {
        Self {
            addr_low: 0,
            segment_selector: SegmentSelector(0),
            options: GateOptions::new(),
            addr_mid: 0,
            addr_high: 0,
            _reserved: 0,
            _gate_type: PhantomData,
        }
    }

    pub unsafe fn set_handler_addr(&mut self, handler: VirtAddr) -> &mut GateOptions<T> {
        let handler = handler.as_u64();
        self.addr_low = handler as u16;
        self.addr_mid = (handler >> 16) as u16;
        self.addr_high = (handler >> 32) as u32;

        self.segment_selector = CS::read();

        self.options.set_present(true);

        return &mut self.options;
    }

    pub fn set_handler_fn(&mut self, handler: F) -> &mut GateOptions<T> {
        unsafe { self.set_handler_addr(handler.to_virt_addr()) }
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
    pub struct PageFaultErrorCode: u64 {
        const PROTECTION_VIOLATION = 1;
        const CAUSED_BY_WRITE = 1 << 1;
        const USER_MODE = 1 << 2;
        const MALFORMED_TABLE = 1 << 3;
        const INSTRUCTION_FETCH = 1 << 4;
        const PROTECTION_KEY = 1 << 5;
        const SHADOW_STACK = 1 << 6;
        const SGX = 1 << 15;
        const RMP = 1 << 31;
    }
}

impl Idt {
    pub fn new() -> Idt {
        debug_assert_eq!(size_of::<Idt>(), 4096);
        Idt {
            divide_error: Gate::default(),
            debug_excepton: Gate::default(),
            nmi_external: Gate::default(),
            break_point: Gate::default(),
            overflow: Gate::default(),
            bound_range_exceeded: Gate::default(),
            invalid_opcode: Gate::default(),
            device_not_available: Gate::default(),
            double_fault: Gate::default(),
            _coprocessor_segment_overrun: Gate::default(),
            invalid_tss: Gate::default(),
            segment_not_present: Gate::default(),
            stack_segment_fault: Gate::default(),
            general_protection: Gate::default(),
            page_fault: Gate::default(),
            _intel_reserved: Gate::default(),
            math_fault: Gate::default(),
            alignment_check: Gate::default(),
            machine_check: Gate::default(),
            simd_exception: Gate::default(),
            virtualization_exception: Gate::default(),
            control_protection_exceptoin: Gate::default(),
            _reserved: [Gate::default(); 10],
            external_interrupts: [Gate::default(); 224],
        }
    }

    pub fn load(&'static self) {
        unsafe {
            lidt(&self.pointer());
        }
    }

    fn pointer(&self) -> DescriptorTablePointer {
        use core::mem::size_of;
        DescriptorTablePointer {
            base: VirtAddr::new(self as *const _ as u64),
            limit: (size_of::<Self>() - 1) as u16,
        }
    }
}

impl Index<usize> for Idt {
    type Output = Gate<NormalHandler, GateInterrupt>;

    fn index(&self, index: usize) -> &Self::Output {
        match index {
            3 => &self.nmi_external,
            i @ 32..=255 => &self.external_interrupts[i - 32],
            _ => panic!("Trying to index into an autistic interrupt vector"), // Get it?, not normal
        }
    }
}

impl IndexMut<usize> for Idt {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        match index {
            3 => &mut self.nmi_external,
            i @ 32..=255 => &mut self.external_interrupts[i - 32],
            _ => panic!("Trying to index into an autistic interrupt vector"), // Get it?, not normal
        }
    }
}
