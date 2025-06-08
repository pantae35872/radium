use alloc::boxed::Box;
use pager::gdt::{Descriptor, Gdt, TaskStateSegment, DOUBLE_FAULT_IST_INDEX};
use pager::registers::{load_tss, CS};

use crate::initialization_context::{End, InitializationContext, Stage3};
use crate::smp::CpuLocalBuilder;
use sentinel::log;

pub fn init_gdt(ctx: &mut InitializationContext<Stage3>) {
    let gdt_initializer = |cpu: &mut CpuLocalBuilder, ctx: &mut InitializationContext<End>, id| {
        let double_fault = ctx
            .stack_allocator()
            .alloc_stack(256)
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
        cpu.gdt(gdt).code_seg(code_selector);
    };
    ctx.local_initializer(|initializer| initializer.register(gdt_initializer));
}
