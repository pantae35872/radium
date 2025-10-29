use core::{
    fmt::Display,
    sync::atomic::{AtomicBool, Ordering},
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use kernel_proc::{def_local, local_builder, local_gen};
use pager::{
    EntryFlags, KERNEL_DIRECT_PHYSICAL_MAP, KERNEL_START, Mapper, PAGE_SIZE,
    address::{Frame, PhysAddr, VirtAddr},
    allocator::FrameAllocator,
    paging::{
        ActivePageTable,
        table::{DirectLevel4, Table},
    },
};
use pager::{
    allocator::linear_allocator::LinearAllocator,
    registers::{Cr3, GsBase, KernelGsBase},
};

use crate::{
    hlt_loop,
    initialization_context::{End, InitializationContext, Stage2, Stage3, select_context},
    interrupt::{
        self, APIC_ID, LAPIC,
        apic::{ApicId, apic_id},
    },
    log,
    memory::{self},
    scheduler::{LOCAL_SCHEDULER, pinned, sleep},
};
use spin::Mutex;

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

        unsafe {
            ctx.context().boot_bridge().kernel_elf().map_permission(
                &mut bootstrap_table.mapper_with_allocator(&mut boot_alloc),
                KERNEL_START,
                ctx.context().boot_bridge().kernel_base(),
            )
        };

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

        LAPIC.inner_mut().send_init_ipi(apic_id, true);
        sleep(10);
        LAPIC.inner_mut().send_init_ipi(apic_id, false);

        for _ in 0..2 {
            LAPIC.inner_mut().send_startup_ipi(apic_id);
            sleep(1);
        }

        while !AP_INITIALIZED.load(Ordering::SeqCst) {
            sleep(1);
        }
        AP_INITIALIZED.store(false, Ordering::SeqCst);
    }
}

static AP_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// The rust entry point for ap cores
///
/// # Safety
/// This should only be called from ap bootstrap trampoline
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_startup(ctx: *const Mutex<InitializationContext<End>>) -> ! {
    // SAFETY: This is safe if not we'll explode
    unsafe { memory::prepare_flags() };
    // SAFETY: This is safe because we called into_raw in the ap startup code and pass through rdi
    // register in the boot.asm
    let ctx = unsafe { Arc::from_raw(ctx) };
    ctx.lock().initialize_current();

    AP_INITIALIZED.store(true, Ordering::SeqCst);

    LOCAL_SCHEDULER.inner_mut().start_scheduling();

    hlt_loop();
}

type LocalInitialize =
    dyn Fn(&mut CpuLocalBuilder, &mut InitializationContext<End>, CoreId) + Send + Sync;
type AfterInitializer = dyn Fn(&mut InitializationContext<End>, CoreId) + Send + Sync;
type AfterBspInitializers = dyn FnOnce(&mut InitializationContext<End>) + Send + Sync;

pub struct LocalInitializer {
    local_initializers_v2: Vec<Box<LocalInitialize>>,
    after_initializers: Vec<Box<AfterInitializer>>,
    after_bsps: Vec<Box<AfterBspInitializers>>,
}

impl LocalInitializer {
    pub const fn new() -> Self {
        Self {
            local_initializers_v2: Vec::new(),
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
        self.local_initializers_v2.push(Box::new(initializer));
    }

    fn initialize_current(&mut self, ctx: &mut InitializationContext<End>) {
        let mut cpu_local_builder = CpuLocalBuilder::new();
        let id = CoreId::from(apic_id());
        log!(Debug, "Initializing cpu: {id}");

        for initializer in self.local_initializers_v2.iter() {
            initializer(&mut cpu_local_builder, ctx, id);
        }

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

impl Default for LocalInitializer {
    fn default() -> Self {
        Self::new()
    }
}

fn init_local(builder: CpuLocalBuilder, core_id: CoreId) {
    let Some(cpu_local) = builder.build() else {
        panic!("Failed to initialize Core: {core_id}");
    };
    let cpu_local = Box::leak(cpu_local.into());
    log!(
        Trace,
        "CORE {core_id} CpuLocal address at: {:#x}",
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
        CPU_ID_TO_APIC_ID
            .get()
            .expect("CPU ID to APIC ID mapping must be initialized core initialization")
            .get(id)?;
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
        .expect("cpu id is not mapped in the mapping")
}

pub fn apic_id_to_core_id(apic_id: usize) -> usize {
    APIC_ID_TO_CPU_ID
        .get()
        .expect("APIC ID to cpu ID mapping must be initialized core initialization")
        .get(apic_id)
        .expect("apic id out of range")
        .expect("apic id is not mapped in the mapping")
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

def_local!(pub static CORE_COUNT: usize);
def_local!(pub static CTX: Arc<Mutex<InitializationContext<End>>>);
local_gen!();

pub fn init_aps(mut ctx: InitializationContext<End>) {
    let ap_initializer = ApInitializer::new(&mut ctx);
    let ctx = Arc::new(Mutex::new(ctx));
    let ctx_cloned = Arc::clone(&ctx);

    ctx.lock().local_initializer(|i| {
        i.register(move |builder, ctx, _id| {
            local_builder!(
                builder,
                CORE_COUNT(ctx.context().processors().len()),
                CTX(ctx_cloned.clone())
            );
        });
    });
    ctx.lock().initialize_current();

    LOCAL_SCHEDULER.inner_mut().spawn(move || {
        pinned(|| {
            let processors = ctx.lock().context().processors().clone();
            processors.iter().copied().for_each(|apic_id| {
                if apic_id == *APIC_ID {
                    return;
                }
                ap_initializer.boot_ap(apic_id, ctx.clone());
            });
            ALL_AP_INITIALIZED.store(true, Ordering::Relaxed);
        })
    });
}
