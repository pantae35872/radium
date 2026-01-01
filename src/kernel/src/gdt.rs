use alloc::boxed::Box;
use kernel_proc::{def_local, local_builder};
use pager::PrivilegeLevel;
use pager::gdt::{DOUBLE_FAULT_IST_INDEX, Descriptor, GENERAL_STACK_INDEX, Gdt, TaskStateSegment};
use pager::registers::{CS, SS, load_tss};

use crate::initialization_context::{InitializationContext, Stage3};
use crate::initialize_guard;
use crate::smp::{ApInitializationContext, CpuLocalBuilder};
use sentinel::log;

def_local!(pub static GDT: &'static pager::gdt::Gdt);
def_local!(pub static KERNEL_CODE_SEG: pager::registers::SegmentSelector);
def_local!(pub static KERNEL_DATA_SEG: pager::registers::SegmentSelector);
def_local!(pub static USER_CODE_SEG: pager::registers::SegmentSelector);
def_local!(pub static USER_DATA_SEG: pager::registers::SegmentSelector);

pub fn init_gdt(ctx: &mut InitializationContext<Stage3>) {
    initialize_guard!();

    let gdt_initializer = |cpu: &mut CpuLocalBuilder, ctx: &ApInitializationContext, id| {
        let double_fault = ctx
            .stack_allocator(|mut s| s.alloc_stack(256))
            .expect("Failed to allocate stack for the double fault handler");
        let general_stack =
            ctx.stack_allocator(|mut s| s.alloc_stack(256)).expect("Failed to allocate general interrupt stack");
        let rsp0_stack = ctx
            .stack_allocator(|mut s| s.alloc_stack(256))
            .expect("Failed to allocate stack for rsp0 privilage change in TSS");

        log!(Trace, "Initializing gdt for core: {id}");
        let tss = Box::leak(TaskStateSegment::new().into());
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = double_fault.top();
        tss.interrupt_stack_table[GENERAL_STACK_INDEX as usize] = general_stack.top();
        tss.privilege_stack_table[0] = rsp0_stack.top();
        let gdt = Box::leak(Gdt::new().into());
        let kernel_code_selector = gdt.add_entry(Descriptor::code_segment(), PrivilegeLevel::Ring0);
        let kernel_data_seg = gdt.add_entry(Descriptor::data_segment(), PrivilegeLevel::Ring0);

        let user_code_selector = gdt.add_entry(Descriptor::code_segment(), PrivilegeLevel::Ring3);
        let user_data_seg = gdt.add_entry(Descriptor::data_segment(), PrivilegeLevel::Ring3);

        let tss_selector = gdt.add_entry(Descriptor::tss_segment(tss), PrivilegeLevel::Ring0);
        gdt.load();
        unsafe {
            CS::set(kernel_code_selector);
            SS::set(kernel_data_seg);
            load_tss(tss_selector);
        }
        local_builder!(
            cpu,
            GDT(gdt),
            KERNEL_CODE_SEG(kernel_code_selector),
            KERNEL_DATA_SEG(kernel_data_seg),
            USER_CODE_SEG(user_code_selector),
            USER_DATA_SEG(user_data_seg),
        );
    };
    ctx.local_initializer(|initializer| initializer.register(gdt_initializer));
}
