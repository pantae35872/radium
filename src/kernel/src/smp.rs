use core::{
    fmt::Display,
    num::NonZeroUsize,
    sync::atomic::{AtomicBool, Ordering},
    u64,
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use pager::{
    address::{Frame, PhysAddr, VirtAddr},
    allocator::FrameAllocator,
    gdt::Gdt,
    paging::{
        table::{DirectLevel4, Table},
        ActivePageTable,
    },
    registers::SegmentSelector,
    EntryFlags, Mapper, KERNEL_DIRECT_PHYSICAL_MAP, PAGE_SIZE,
};
use pager::{
    allocator::linear_allocator::LinearAllocator,
    registers::{Cr3, GsBase, KernelGsBase},
};
use spin::Mutex;

use crate::{
    hlt_loop,
    initialization_context::{select_context, End, InitializationContext, Stage2, Stage3},
    interrupt::{
        self,
        apic::{apic_id, ApicId, LocalApic},
        idt::Idt,
    },
    log,
    memory::{self},
    println,
    scheduler::{sleep, LocalScheduler},
    serial_println,
};

pub const MAX_CPU: usize = 64;

pub static APIC_ID_TO_CPU_ID: OnceCell<[Option<usize>; MAX_CPU]> = OnceCell::uninit();
pub static CPU_ID_TO_APIC_ID: OnceCell<[Option<usize>; MAX_CPU]> = OnceCell::uninit();
pub static ALL_AP_INITIALIZED: AtomicBool = AtomicBool::new(false);
static BSP_CORE_ID: OnceCell<CoreId> = OnceCell::uninit();

pub const TRAMPOLINE_START: PhysAddr = PhysAddr::new(0x7000);
pub const TRAMPOLINE_END: PhysAddr = PhysAddr::new(0x9000);

unsafe extern "C" {
    static __trampoline_start: u8;
    static __trampoline_end: u8;
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
    fn new(ctx: &mut InitializationContext<End>) -> Self {
        let trampoline_size = unsafe {
            &__trampoline_end as *const u8 as usize - &__trampoline_start as *const u8 as usize
        };

        // Safety we already allocted this at the bootloader
        let mut boot_alloc =
            unsafe { LinearAllocator::new(PhysAddr::new(0x100000), 64 * PAGE_SIZE as usize) };
        // SAFETY: We know that the bootloader is not used anymore
        unsafe { boot_alloc.reset() };

        unsafe { ctx.mapper().identity_map_object(&boot_alloc.mappings()) };
        unsafe {
            core::ptr::write_bytes(
                boot_alloc.original_start().as_u64() as *mut u8,
                0,
                boot_alloc.size(),
            );
        }

        let p4_table = boot_alloc
            .allocate_frame()
            .expect("Failed to allocate frame for temporary early boot");
        let mut bootstrap_table = unsafe {
            ActivePageTable::<DirectLevel4>::new_custom(
                p4_table.start_address().as_u64() as *mut Table<DirectLevel4>
            )
        };

        bootstrap_table.virtually_map_object(
            ctx.context().boot_bridge().kernel_elf(),
            ctx.context().boot_bridge().kernel_base(),
            &mut boot_alloc,
        );

        unsafe {
            bootstrap_table
                .mapper_with_allocator(&mut boot_alloc)
                .identity_map_by_size(
                    PhysAddr::new(0x7000).into(),
                    (PAGE_SIZE * 4) as usize,
                    EntryFlags::WRITABLE,
                )
        };

        unsafe {
            core::ptr::copy(
                &__trampoline_start as *const u8,
                (KERNEL_DIRECT_PHYSICAL_MAP.as_u64() + 0x8000) as *mut u8,
                trampoline_size,
            )
        };

        Self {
            ap_bootstrap_page_table: p4_table,
        }
    }

    fn prepare_stack_and_info(&self, ctx: Arc<Mutex<InitializationContext<End>>>) {
        let ctx_ap = Arc::clone(&ctx);
        let mut ctx = ctx.lock();
        let stack = ctx
            .stack_allocator()
            .alloc_stack(256)
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
                (KERNEL_DIRECT_PHYSICAL_MAP.as_u64() + 0x7000) as *mut SmpInitializationData,
                1,
            );
        }
    }

    fn boot_ap(&self, apic_id: ApicId, ctx: Arc<Mutex<InitializationContext<End>>>) {
        self.prepare_stack_and_info(ctx);
        assert!(!AP_INITIALIZED.load(Ordering::SeqCst));

        cpu_local().lapic().send_init_ipi(apic_id, true);
        sleep(10);
        cpu_local().lapic().send_init_ipi(apic_id, false);

        for _ in 0..2 {
            cpu_local().lapic().send_startup_ipi(apic_id);
            sleep(1);
        }

        while !AP_INITIALIZED.load(Ordering::SeqCst) {
            sleep(1);
        }
        AP_INITIALIZED.store(false, Ordering::SeqCst);
    }
}

static AP_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[unsafe(no_mangle)]
pub extern "C" fn ap_startup(ctx: *const Mutex<InitializationContext<End>>) -> ! {
    // SAFETY: This is safe if not we'll explode
    unsafe { memory::prepare_flags() };
    // SAFETY: This is safe because we called into_raw in the ap startup code and pass through rdi
    // register in the boot.asm
    let ctx = unsafe { Arc::from_raw(ctx) };
    ctx.lock().initialize_current();

    AP_INITIALIZED.store(true, Ordering::SeqCst);

    cpu_local().local_scheduler().start_scheduling();

    hlt_loop();
}

pub struct LocalInitializer {
    local_initializers: Vec<
        Box<dyn Fn(&mut CpuLocalBuilder, &mut InitializationContext<End>, CoreId) + Send + Sync>,
    >,
    after_initializers: Vec<Box<dyn Fn(&mut InitializationContext<End>, CoreId) + Send + Sync>>,
    after_bsps: Vec<Box<dyn FnOnce(&mut InitializationContext<End>) + Send + Sync>>,
}

impl LocalInitializer {
    pub const fn new() -> Self {
        Self {
            local_initializers: Vec::new(),
            after_initializers: Vec::new(),
            after_bsps: Vec::new(),
        }
    }

    pub fn after_bsp(
        &mut self,
        initializer: impl FnOnce(&mut InitializationContext<End>) + Send + Sync + 'static,
    ) {
        self.after_bsps.push(Box::new(initializer));
    }

    pub fn register_after(
        &mut self,
        initializer: impl Fn(&mut InitializationContext<End>, CoreId) + Send + Sync + 'static,
    ) {
        self.after_initializers.push(Box::new(initializer));
    }

    pub fn register(
        &mut self,
        initializer: impl Fn(&mut CpuLocalBuilder, &mut InitializationContext<End>, CoreId)
            + Send
            + Sync
            + 'static,
    ) {
        self.local_initializers.push(Box::new(initializer));
    }

    fn initialize_current(&mut self, ctx: &mut InitializationContext<End>) {
        let mut cpu_local_builder = CpuLocalBuilder::new();
        let id = CoreId::from(apic_id());
        log!(Debug, "Initializing cpu: {id}");

        self.local_initializers
            .iter()
            .for_each(|e| e(&mut cpu_local_builder, ctx, id));

        init_local(cpu_local_builder, id);

        // We enable the interrupts after we're sure that the local has been initialized
        interrupt::enable();

        if BSP_CORE_ID.get().is_some_and(|bsp_id| *bsp_id == id) {
            log!(Debug, "initialization bsp, bsp processor id: {id}");
            while let Some(f) = self.after_bsps.pop() {
                f(ctx);
            }
        }

        self.after_initializers.iter().for_each(|e| e(ctx, id));
    }
}

pub struct CpuLocal {
    core_id: CoreId,
    apic_id: ApicId,
    lapic: LocalApic,
    code_seg: SegmentSelector,
    thread_id: usize,
    ticks_per_ms: Option<usize>,
    local_scheduler: LocalScheduler,
    ctx: Arc<Mutex<InitializationContext<End>>>,
    pub last_interrupt_no: u8,
    pub is_in_isr: bool,
    core_count: usize,
    idt: &'static Idt,
    gdt: &'static Gdt,
}

impl CpuLocal {
    #[inline]
    pub fn core_count(&self) -> usize {
        self.core_count
    }

    #[inline]
    pub fn core_id(&self) -> CoreId {
        self.core_id
    }

    #[inline]
    pub fn apic_id(&self) -> ApicId {
        self.apic_id
    }

    pub fn set_tpms(&mut self, tpms: NonZeroUsize) {
        self.ticks_per_ms = Some(tpms.get());
    }

    pub fn ticks_per_ms(&self) -> usize {
        self.ticks_per_ms.expect("TPMS is not calibrated")
    }

    pub fn local_scheduler(&mut self) -> &mut LocalScheduler {
        &mut self.local_scheduler
    }

    pub fn ctx(&self) -> &Mutex<InitializationContext<End>> {
        &self.ctx
    }

    pub fn code_seg(&self) -> SegmentSelector {
        self.code_seg
    }

    /// Setting the tid is unsafe, and can cause undefined behaviour
    pub unsafe fn set_tid(&mut self, id: usize) {
        self.thread_id = id;
    }

    pub fn current_thread_id(&self) -> usize {
        self.thread_id
    }

    pub fn lapic(&mut self) -> &mut LocalApic {
        &mut self.lapic
    }
}

pub struct CpuLocalBuilder {
    lapic: Option<LocalApic>,
    gdt: Option<&'static Gdt>,
    idt: Option<&'static Idt>,
    code_seg: Option<SegmentSelector>,
    core_count: Option<usize>,
    initialization_contex: Option<Arc<Mutex<InitializationContext<End>>>>,
    local_scheduler: Option<LocalScheduler>,
}

impl CpuLocalBuilder {
    pub const fn new() -> Self {
        Self {
            lapic: None,
            idt: None,
            gdt: None,
            code_seg: None,
            initialization_contex: None,
            local_scheduler: None,
            core_count: None,
        }
    }

    pub fn ctx(&mut self, ctx: Arc<Mutex<InitializationContext<End>>>) -> &mut Self {
        self.initialization_contex = Some(ctx);
        self
    }

    pub fn scheduler(&mut self, scheduler: LocalScheduler) -> &mut Self {
        self.local_scheduler = Some(scheduler);
        self
    }

    pub fn core_count(&mut self, core_count: usize) -> &mut Self {
        self.core_count = Some(core_count);
        self
    }

    pub fn code_seg(&mut self, code_seg: SegmentSelector) -> &mut Self {
        self.code_seg = Some(code_seg);
        self
    }

    pub fn lapic(&mut self, apic: LocalApic) -> &mut Self {
        self.lapic = Some(apic);
        self
    }

    pub fn idt(&mut self, idt: &'static Idt) -> &mut Self {
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
                core_id: lapic.id().into(),
                idt: self.idt?,
                apic_id: lapic.id(),
                code_seg: self.code_seg?,
                ctx: self.initialization_contex?,
                local_scheduler: self.local_scheduler?,
                last_interrupt_no: 0,
                ticks_per_ms: None,
                thread_id: 0,
                is_in_isr: false,
                core_count: self.core_count?,
                lapic,
                gdt: self.gdt?,
            }
            .into(),
        ))
    }
}

fn init_local(builder: CpuLocalBuilder, core_id: CoreId) {
    let cpu_local = match builder.build() {
        Some(e) => e,
        None => {
            log!(Error, "Failed to initialize Core: {core_id}");
            interrupt::disable();
            hlt_loop();
        }
    };
    log!(
        Trace,
        "CORE {core_id} Local address at: {:#x}",
        cpu_local as *const CpuLocal as u64
    );
    let ptr = Box::leak((cpu_local as *const CpuLocal as u64).into());
    // SAFETY: This is safe beacuse we correctly allocated the ptr on the line above
    unsafe {
        KernelGsBase::write(VirtAddr::new(ptr as *const u64 as u64));
        GsBase::write(VirtAddr::new(ptr as *const u64 as u64));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct CoreId(usize);

impl CoreId {
    pub fn new(id: usize) -> Option<Self> {
        if CPU_ID_TO_APIC_ID
            .get()
            .expect("CPU ID to APIC ID mapping must be initialized core initialization")
            .get(id)
            .is_none()
        {
            return None;
        }
        Some(Self(id))
    }

    #[inline]
    pub fn id(&self) -> usize {
        self.0
    }
}

impl Display for CoreId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ApicId> for CoreId {
    fn from(apic_id: ApicId) -> Self {
        Self(apic_id_to_core_id(apic_id.id()))
    }
}

pub fn core_id_to_apic_id(core_id: usize) -> usize {
    CPU_ID_TO_APIC_ID
        .get()
        .expect("CPU ID to APIC ID mapping must be initialized core initialization")
        .get(core_id)
        .expect("cpu id out of range")
        .expect("cpu id is not mapped in the mapping") as usize
}

pub fn apic_id_to_core_id(apic_id: usize) -> usize {
    APIC_ID_TO_CPU_ID
        .get()
        .expect("APIC ID to cpu ID mapping must be initialized core initialization")
        .get(apic_id)
        .expect("apic id out of range")
        .expect("apic id is not mapped in the mapping") as usize
}

/// Check if the cpu local is initialized or not
pub fn cpu_local_avaiable() -> bool {
    !(KernelGsBase::read().is_null() || GsBase::read().is_null())
}

/// Get the cpu local
///
/// Panics if the cpu local is not initialized, can be checked by cpu_local_avaiable function
#[inline(always)]
pub fn cpu_local() -> &'static mut CpuLocal {
    let ptr: *mut CpuLocal;
    if KernelGsBase::read().is_null() || GsBase::read().is_null() {
        panic!("Trying to access cpu local while, it's has not been initialized");
    }
    unsafe {
        core::arch::asm!("mov {}, gs:0", out(reg) ptr);
    }
    unsafe { &mut *ptr }
}

select_context! {
    (Stage3, End) => {
        pub fn local_initializer(&mut self, f: impl FnOnce(&mut LocalInitializer)) {
            let mut initializer = self.context_mut().local_initializer.take().unwrap();
            f(&mut initializer);
            self.context_mut().local_initializer = Some(initializer);
        }
    }
}

impl InitializationContext<End> {
    pub fn initialize_current(&mut self) {
        let mut initializer = self.context_mut().local_initializer.take().unwrap();
        initializer.initialize_current(self);
        self.context_mut().local_initializer = Some(initializer);
    }
}

pub fn init(ctx: InitializationContext<Stage2>) -> InitializationContext<Stage3> {
    let processors = ctx.context().processors();
    let mut cpu_id_to_apic_id = [None; MAX_CPU];
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
                BSP_CORE_ID.init_once(|| CoreId(current_id));
            }
            id[apic_id.id()] = Some(current_id);
            cpu_id_to_apic_id[current_id] = Some(apic_id.id());
            current_id += 1;
        });
        id
    });
    CPU_ID_TO_APIC_ID.init_once(|| cpu_id_to_apic_id);
    ctx.next(Some(LocalInitializer::new()))
}

pub fn init_aps(mut ctx: InitializationContext<End>) {
    let ap_initializer = ApInitializer::new(&mut ctx);
    let ctx = Arc::new(Mutex::new(ctx));
    let ctx_cloned = Arc::clone(&ctx);
    ctx.lock().local_initializer(|i| {
        i.register(move |builder, ctx, _id| {
            builder
                .core_count(ctx.context().processors().len())
                .ctx(ctx_cloned.clone());
        })
    });
    ctx.lock().initialize_current();

    cpu_local().local_scheduler().spawn(move || {
        let processors = ctx.lock().context().processors().clone();
        processors.iter().copied().for_each(|apic_id| {
            if apic_id == cpu_local().apic_id() {
                return;
            }
            ap_initializer.boot_ap(apic_id, ctx.clone());
        });
        ALL_AP_INITIALIZED.store(true, Ordering::Relaxed);
    });
}
