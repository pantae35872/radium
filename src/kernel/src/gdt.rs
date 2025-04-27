use alloc::boxed::Box;
use pager::address::VirtAddr;
use pager::gdt::{Descriptor, Gdt, TaskStateSegment, DOUBLE_FAULT_IST_INDEX};
use pager::registers::{lgdt, load_tss, DescriptorTablePointer, SegmentSelector, CS};
use pager::PrivilegeLevel;

use crate::initialization_context::{InitializationContext, Phase3};
use crate::log;
use crate::smp::CpuLocalBuilder;

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
