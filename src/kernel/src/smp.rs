use core::{
    sync::atomic::{AtomicBool, Ordering},
    u64,
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use pager::registers::{Cr3, GsBase, KernelGsBase};
use pager::{
    address::{Frame, PhysAddr, VirtAddr},
    EntryFlags, Mapper,
};
use raw_cpuid::CpuId;
use spin::Mutex;
use x86_64::{
    instructions::{self},
    structures::idt::InterruptDescriptorTable,
};

use crate::{
    gdt::Gdt,
    hlt_loop,
    initialization_context::{InitializationContext, Phase2, Phase3},
    interrupt::{apic::LocalApic, TIMER_COUNT},
    log,
    memory::{
        self,
        allocator::linear_allocator::LinearAllocator,
        paging::{
            table::{DirectLevel4, Table},
            ActivePageTable,
        },
        FrameAllocator,
    },
};

pub const MAX_CPU: usize = 64;

static APIC_ID_TO_CPU_ID: OnceCell<Mutex<[Option<usize>; MAX_CPU]>> = OnceCell::uninit();
static BSP_CPU_ID: OnceCell<usize> = OnceCell::uninit();

pub const TRAMPOLINE_START: PhysAddr = PhysAddr::new(0x7000);
pub const TRAMPOLINE_END: PhysAddr = PhysAddr::new(0x9000);

unsafe extern "C" {
    static __trampoline_start: u8;
    static __trampoline_end: u8;
    static early_alloc: u8;
}

#[repr(C)]
struct SmpInitializationData {
    page_table: u32,
    _padding: u32, // Just to make it clear to me
    real_page_table: u64,
    stack: VirtAddr,
    stack_bottom: VirtAddr,
    ap_context: VirtAddr,
}

pub struct ApInitializer {
    ap_bootstrap_page_table: Frame,
}

impl ApInitializer {
    fn new(ctx: &mut InitializationContext<Phase3>) -> Self {
        let trampoline_size = unsafe {
            &__trampoline_end as *const u8 as usize - &__trampoline_start as *const u8 as usize
        };

        unsafe {
            core::ptr::write_bytes(&early_alloc as *const u8 as *mut u8, 0, 4096 * 64);
        }

        // We reuse the early boot memory we used to bootstrap this core
        // We know it's below 4GB range because our kernel is at 1M
        // so it's fits in 32-bit register
        let mut boot_alloc = unsafe {
            LinearAllocator::new(PhysAddr::new(&early_alloc as *const u8 as u64), 4096 * 64)
        };

        let p4_table = boot_alloc
            .allocate_frame()
            .expect("Failed to allocate frame for temporary early boot");
        let mut bootstrap_table = unsafe {
            ActivePageTable::<DirectLevel4>::new_custom(
                p4_table.start_address().as_u64() as *mut Table<DirectLevel4>
            )
        };

        bootstrap_table.identity_map_object(ctx.context().boot_bridge(), &mut boot_alloc);

        unsafe {
            bootstrap_table.identity_map_range(
                Frame::containing_address(PhysAddr::new(0x7000)),
                Frame::containing_address(PhysAddr::new(0x8000)),
                EntryFlags::WRITABLE | EntryFlags::PRESENT,
                &mut boot_alloc,
            );

            ctx.mapper().identity_map_range(
                Frame::containing_address(PhysAddr::new(0x7000)),
                Frame::containing_address(PhysAddr::new(0x8000)),
                EntryFlags::WRITABLE,
            );
        };

        unsafe {
            core::ptr::copy(
                &__trampoline_start as *const u8,
                0x8000 as *mut u8,
                trampoline_size,
            )
        };

        Self {
            ap_bootstrap_page_table: p4_table,
        }
    }

    fn prepare_stack_and_info(&self, ctx: Arc<Mutex<InitializationContext<Phase3>>>) {
        let ctx_ap = Arc::clone(&ctx);
        let mut ctx = ctx.lock();
        let stack = ctx
            .stack_allocator()
            .alloc_stack(8)
            .expect("Failed to allocate stack for ap");

        let data = SmpInitializationData {
            page_table: self.ap_bootstrap_page_table.start_address().as_u64() as u32,
            _padding: 0,
            real_page_table: Cr3::read().0.start_address().as_u64(),
            stack: stack.top(),
            stack_bottom: stack.bottom(),
            ap_context: VirtAddr::new(Arc::into_raw(ctx_ap) as u64),
        };

        log!(Trace, "AP Bootstrap page table at {:#x}", data.page_table);
        log!(
            Trace,
            "AP Bootstrap stack, Top: {:#x}, Bottom: {:#x}",
            data.stack,
            data.stack_bottom
        );

        unsafe {
            core::ptr::copy(
                &data as *const SmpInitializationData,
                0x7000 as *mut SmpInitializationData,
                1,
            );
        }
    }

    fn boot_ap(&self, apic_id: usize, ctx: Arc<Mutex<InitializationContext<Phase3>>>) {
        self.prepare_stack_and_info(ctx);
        assert!(!AP_INITIALIZED.load(Ordering::SeqCst));

        cpu_local().lapic().send_init_ipi(apic_id);

        clunky_wait(10);
        for _ in 0..2 {
            cpu_local().lapic().send_startup_ipi(apic_id);
            clunky_wait(1);
        }

        while !AP_INITIALIZED.load(Ordering::SeqCst) {
            clunky_wait(1);
        }
        AP_INITIALIZED.store(false, Ordering::SeqCst);
    }
}

static AP_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[unsafe(no_mangle)]
pub extern "C" fn ap_startup(ctx: *const Mutex<InitializationContext<Phase3>>) -> ! {
    // SAFETY: This is safe if not we'll explode
    unsafe { memory::prepare_flags() };
    // SAFETY: This is safe because we called into_raw in the ap startup code and pass through rdi
    // register in the boot.asm
    let ctx = unsafe { Arc::from_raw(ctx) };
    ctx.lock().initialize_current();
    AP_INITIALIZED.store(true, Ordering::SeqCst);

    hlt_loop();
}

pub struct LocalInitializer {
    local_initializers: Vec<
        Box<dyn Fn(&mut CpuLocalBuilder, &mut InitializationContext<Phase3>, usize) + Send + Sync>,
    >,
    after_bsps: Vec<Box<dyn FnOnce(&CpuLocal) + Send + Sync>>,
}

pub struct CpuLocal {
    cpu_id: usize,
    apic_id: usize,
    lapic: LocalApic,
    idt: &'static InterruptDescriptorTable,
    gdt: &'static Gdt,
}

pub struct CpuLocalBuilder {
    lapic: Option<LocalApic>,
    gdt: Option<&'static Gdt>,
    idt: Option<&'static InterruptDescriptorTable>,
}

impl CpuLocalBuilder {
    pub const fn new() -> Self {
        Self {
            lapic: None,
            idt: None,
            gdt: None,
        }
    }

    pub fn lapic(&mut self, apic: LocalApic) -> &mut Self {
        self.lapic = Some(apic);
        self
    }

    pub fn idt(&mut self, idt: &'static InterruptDescriptorTable) -> &mut Self {
        self.idt = Some(idt);
        self
    }

    pub fn gdt(&mut self, gdt: &'static Gdt) -> &mut Self {
        self.gdt = Some(gdt);
        self
    }

    fn build(self) -> Option<&'static CpuLocal> {
        let lapic = self.lapic?;
        Some(Box::<CpuLocal>::leak(
            CpuLocal {
                cpu_id: apic_id_to_cpu_id(lapic.id()),
                idt: self.idt?,
                apic_id: lapic.id(),
                lapic,
                gdt: self.gdt?,
            }
            .into(),
        ))
    }
}

impl LocalInitializer {
    pub const fn new() -> Self {
        Self {
            local_initializers: Vec::new(),
            after_bsps: Vec::new(),
        }
    }

    pub fn after_bsp(&mut self, initializer: impl FnOnce(&CpuLocal) + Send + Sync + 'static) {
        self.after_bsps.push(Box::new(initializer));
    }

    pub fn register(
        &mut self,
        initializer: impl Fn(&mut CpuLocalBuilder, &mut InitializationContext<Phase3>, usize)
            + Send
            + Sync
            + 'static,
    ) {
        self.local_initializers.push(Box::new(initializer));
    }

    fn initialize_current(&mut self, ctx: &mut InitializationContext<Phase3>) {
        let mut cpu_local_builder = CpuLocalBuilder::new();
        let id = apic_id_to_cpu_id(apic_id());
        log!(Debug, "Initializing cpu: {id}");

        self.local_initializers
            .iter()
            .for_each(|e| e(&mut cpu_local_builder, ctx, id));

        init_local(cpu_local_builder, id);

        if BSP_CPU_ID.get().is_some_and(|bsp_id| *bsp_id == id) {
            log!(Debug, "initialization bsp, bsp processor id: {id}");
            while let Some(f) = self.after_bsps.pop() {
                f(&cpu_local());
            }
        }
    }
}

impl CpuLocal {
    pub fn apic_id(&self) -> usize {
        self.apic_id
    }

    pub fn cpu_id(&self) -> usize {
        self.cpu_id
    }

    pub fn lapic(&mut self) -> &mut LocalApic {
        &mut self.lapic
    }
}

fn apic_id() -> usize {
    CpuId::new()
        .get_feature_info()
        .unwrap()
        .initial_local_apic_id() as usize
}

fn init_local(builder: CpuLocalBuilder, cpu_id: usize) {
    let cpu_local = match builder.build() {
        Some(e) => e,
        None => {
            log!(Error, "Failed to initialize CPU: {cpu_id}");
            return;
        }
    };
    log!(
        Trace,
        "CPU {cpu_id} Local address at: {:#x}",
        cpu_local as *const CpuLocal as u64
    );
    let ptr = Box::leak((cpu_local as *const CpuLocal as u64).into());
    // SAFETY: This is safe beacuse we correctly allocated the ptr on the line above
    unsafe {
        KernelGsBase::write(VirtAddr::new(ptr as *const u64 as u64));
        GsBase::write(VirtAddr::new(ptr as *const u64 as u64));
    }
}

fn apic_id_to_cpu_id(apic_id: usize) -> usize {
    APIC_ID_TO_CPU_ID
        .get()
        .expect("APIC ID to cpu ID mapping must be initialized core initialization")
        .lock()
        .get(apic_id)
        .expect("apic id out of range")
        .expect("apic id is not mapped in the mapping") as usize
}

#[inline(always)]
pub fn cpu_local() -> &'static mut CpuLocal {
    let ptr: *mut CpuLocal;
    unsafe {
        core::arch::asm!("mov {}, gs:0", out(reg) ptr);
    }
    unsafe { &mut *ptr }
}

fn clunky_wait(ms: usize) {
    let end_time = TIMER_COUNT.load(Ordering::Relaxed) + ms;
    while TIMER_COUNT.load(Ordering::Relaxed) < end_time {
        instructions::hlt();
    }
}

impl InitializationContext<Phase3> {
    pub fn initialize_current(&mut self) {
        let mut initializer = self.context_mut().local_initializer.take().unwrap();
        initializer.initialize_current(self);
        self.context_mut().local_initializer = Some(initializer);
    }

    pub fn local_initializer(&mut self, f: impl FnOnce(&mut LocalInitializer)) {
        let mut initializer = self.context_mut().local_initializer.take().unwrap();
        f(&mut initializer);
        self.context_mut().local_initializer = Some(initializer);
    }
}

pub fn init(ctx: InitializationContext<Phase2>) -> InitializationContext<Phase3> {
    let processors = ctx.context().processors();
    APIC_ID_TO_CPU_ID.init_once(|| {
        let mut id = [None; MAX_CPU];
        let mut current_id = 0;
        let bsp_apic_id = apic_id();
        processors.iter().copied().for_each(|apic_id| {
            log!(
                Info,
                "Found Processor with apic: {apic_id}, Mapping it to CPU ID: {current_id}"
            );
            if apic_id == bsp_apic_id {
                BSP_CPU_ID.init_once(|| current_id);
            }
            id[apic_id] = Some(current_id);
            current_id += 1;
        });
        id.into()
    });
    ctx.next(Some(LocalInitializer::new()))
}

pub fn init_aps(mut ctx: InitializationContext<Phase3>) {
    ctx.initialize_current();

    let ap_initializer = ApInitializer::new(&mut ctx);
    let ctx = Arc::new(Mutex::new(ctx));
    let processors = ctx.lock().context().processors().clone();
    processors.iter().copied().for_each(|apic_id| {
        if apic_id == cpu_local().apic_id() {
            return;
        }
        ap_initializer.boot_ap(apic_id, ctx.clone());
    });
}
