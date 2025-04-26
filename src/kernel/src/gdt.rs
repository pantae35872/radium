use alloc::boxed::Box;
use pager::address::VirtAddr;
use pager::registers::{lgdt, load_tss, DescriptorTablePointer, SegmentSelector, CS};
use pager::PrivilegeLevel;

use crate::initialization_context::{InitializationContext, Phase3};
use crate::log;
use crate::smp::CpuLocalBuilder;

pub struct Gdt {
    table: [u64; 8],
    next_free: usize,
}

impl Gdt {
    pub fn new() -> Gdt {
        Gdt {
            table: [0; 8],
            next_free: 1,
        }
    }

    pub fn add_entry(&mut self, entry: Descriptor) -> SegmentSelector {
        let index = match entry {
            Descriptor::UserSegment(value) => self.push(value),
            Descriptor::SystemSegment(value_low, value_high) => {
                let index = self.push(value_low);
                self.push(value_high);
                index
            }
        };
        SegmentSelector::new(index as u16, PrivilegeLevel::Ring0)
    }

    pub fn load(&'static self) {
        use core::mem::size_of;

        let ptr = DescriptorTablePointer {
            base: VirtAddr::new(self.table.as_ptr() as u64),
            limit: (self.table.len() * size_of::<u64>() - 1) as u16,
        };

        unsafe { lgdt(&ptr) };
    }

    pub fn limit(&self) -> u16 {
        (self.table.len() * size_of::<u64>() - 1) as u16
    }

    fn push(&mut self, value: u64) -> usize {
        if self.next_free < self.table.len() {
            let index = self.next_free;
            self.table[index] = value;
            self.next_free += 1;
            index
        } else {
            panic!("GDT full");
        }
    }
}

pub fn init_gdt(ctx: &mut InitializationContext<Phase3>) {
    let gdt_initializer =
        |cpu: &mut CpuLocalBuilder, ctx: &mut InitializationContext<Phase3>, id| {
            let double_fault = ctx
                .stack_allocator()
                .alloc_stack(1)
                .expect("Failed to allocator stack for double fault handler");
            log!(Trace, "Initializing gdt for core: {id}");
            log!(
                Debug,
                "Double fault handler stack, Top: {:#x}, Bottom: {:#x}",
                double_fault.top(),
                double_fault.bottom()
            );
            let tss = Box::leak(TaskStateSegment::new().into());
            tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = double_fault.top();
            let gdt = Box::leak(Gdt::new().into());
            let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
            let tss_selector = gdt.add_entry(Descriptor::tss_segment(tss));
            gdt.load();
            unsafe {
                CS::set(code_selector);
                load_tss(tss_selector);
            }
            cpu.gdt(gdt);
        };
    ctx.local_initializer(|initializer| initializer.register(gdt_initializer));
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed(4))]
pub struct TaskStateSegment {
    _reserved_1: u32,
    pub privilege_stack_table: [VirtAddr; 3],
    _reserved_2: u64,
    pub interrupt_stack_table: [VirtAddr; 7],
    _reserved_3: u64,
    _reserved_4: u16,
    pub iomap_base: u16,
}

impl TaskStateSegment {
    pub const fn new() -> TaskStateSegment {
        TaskStateSegment {
            privilege_stack_table: [VirtAddr::null(); 3],
            interrupt_stack_table: [VirtAddr::null(); 7],
            iomap_base: size_of::<TaskStateSegment>() as u16,
            _reserved_1: 0,
            _reserved_2: 0,
            _reserved_3: 0,
            _reserved_4: 0,
        }
    }
}

pub enum Descriptor {
    UserSegment(u64),
    SystemSegment(u64, u64),
}
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

impl Descriptor {
    pub fn kernel_code_segment() -> Descriptor {
        let flags = DescriptorFlags::USER_SEGMENT
            | DescriptorFlags::PRESENT
            | DescriptorFlags::EXECUTABLE
            | DescriptorFlags::LONG_MODE;
        Descriptor::UserSegment(flags.bits())
    }

    pub fn tss_segment(tss: &'static TaskStateSegment) -> Descriptor {
        use bit_field::BitField;
        use core::mem::size_of;

        let ptr = tss as *const _ as u64;

        let mut low = DescriptorFlags::PRESENT.bits();
        // base
        low.set_bits(16..40, ptr.get_bits(0..24));
        low.set_bits(56..64, ptr.get_bits(24..32));
        // limit (the `-1` in needed since the bound is inclusive)
        low.set_bits(0..16, (size_of::<TaskStateSegment>() - 1) as u64);
        // type (0b1001 = available 64-bit tss)
        low.set_bits(40..44, 0b1001);

        let mut high = 0;
        high.set_bits(0..32, ptr.get_bits(32..64));

        Descriptor::SystemSegment(low, high)
    }
}

bitflags! {
    pub struct DescriptorFlags: u64 {
        const CONFORMING        = 1 << 42;
        const EXECUTABLE        = 1 << 43;
        const USER_SEGMENT      = 1 << 44;
        const PRESENT           = 1 << 47;
        const LONG_MODE         = 1 << 53;
    }
}
