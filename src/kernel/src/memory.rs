use core::cell::RefCell;

use alloc::sync::Arc;
use allocator::{area_allocator::AreaAllocator, buddy_allocator::BuddyAllocator};
use bootbridge::{BootBridge, MemoryType, RawData};
use kernel_proc::{def_local, local_builder};
use pager::{
    EntryFlags, PAGE_SIZE,
    address::{Frame, Page, PhysAddr, VirtAddr},
    allocator::FrameAllocator,
    paging::{
        ActivePageTable, InactivePageCopyOption, InactivePageTable, TableManipulationContext,
        mapper::{Mapper, MapperWithAllocator, TopLevelP4, TopLevelRecurse},
        table::{RecurseLevel4, RecurseLevel4LowerHalf, RecurseLevel4UpperHalf},
        temporary_page::TemporaryPage,
    },
    registers::{Cr0, Cr4, Cr4Flags, Efer, Xcr0},
    virt_addr_alloc,
};
use raw_cpuid::CpuId;
use spin::Mutex;
use stack_allocator::StackAllocator;

use crate::{
    DWARF_DATA,
    driver::acpi::Acpi,
    initialization_context::{InitializationContext, Stage0, Stage1, Stage4, select_context},
    initialize_guard, log,
};

pub use self::paging::remap_the_kernel;

pub mod allocator;
pub mod paging;
pub mod stack_allocator;

pub const MAX_ALIGN: usize = 8192;
pub const STACK_ALLOC_SIZE: u64 = 32768;

def_local!(pub static ACTIVE_TABLE_UPPER: Arc<Mutex<ActivePageTable<RecurseLevel4UpperHalf>>>);
def_local!(pub static ACTIVE_TABLE_LOWER: RefCell<ActivePageTable<RecurseLevel4LowerHalf>>);

def_local!(pub static STACK_ALLOCATOR: Arc<Mutex<StackAllocator>>);
def_local!(pub static BUDDY_ALLOCATOR: Arc<Mutex<BuddyAllocator<64>>>);
def_local!(pub static TEMPORARY_PAGE: Arc<Mutex<TemporaryPage>>);

pub fn stack_allocator<R>(
    f: impl FnOnce(WithMapper<StackAllocator, BuddyAllocator, RecurseLevel4UpperHalf>) -> R,
) -> R {
    let mut stack_allocator = STACK_ALLOCATOR.lock();
    let mut table = ACTIVE_TABLE_UPPER.lock();
    f(stack_allocator.with_table(&mut *table, &mut BUDDY_ALLOCATOR.lock()))
}

pub fn switch_lower_half(with: InactivePageTable<RecurseLevel4LowerHalf>) -> InactivePageTable<RecurseLevel4LowerHalf> {
    let upper = &mut *ACTIVE_TABLE_UPPER.lock();
    let allocator = &mut *BUDDY_ALLOCATOR.lock();
    let temporary_page = &mut TEMPORARY_PAGE.lock();
    // SAFETY: Switching the user level4 is completely safe, i think
    unsafe {
        ACTIVE_TABLE_LOWER.borrow_mut().switch(
            &mut TableManipulationContext { temporary_page, allocator, temporary_page_mapper: Some(upper) },
            with,
        )
    }
}

/// Just a helper See [`ActivePageTable::create_mappings`] for more info
///
/// # Safety
/// See [`ActivePageTable::create_mappings`].
pub unsafe fn copy_mappings<From: TopLevelRecurse, To: TopLevelRecurse>(
    options: InactivePageCopyOption,
    copy_from: &InactivePageTable<From>,
) -> InactivePageTable<To> {
    let upper = &mut *ACTIVE_TABLE_UPPER.lock();
    let allocator = &mut *BUDDY_ALLOCATOR.lock();
    let temporary_page = &mut TEMPORARY_PAGE.lock();
    // SAFETY: This is just a helper function, the options contract are uphold by the caller
    unsafe {
        upper.copy_mappings_from(
            &mut TableManipulationContext { temporary_page, allocator, temporary_page_mapper: None },
            options,
            copy_from,
        )
    }
}

/// Just a helper See [`ActivePageTable::create_mappings`] for more info
///
/// # Safety
/// See [`ActivePageTable::create_mappings`].
pub unsafe fn create_mappings_lower<F>(
    f: F,
    options: InactivePageCopyOption,
) -> InactivePageTable<RecurseLevel4LowerHalf>
where
    F: FnOnce(&mut Mapper<RecurseLevel4LowerHalf>, &mut BuddyAllocator),
{
    let upper = &mut *ACTIVE_TABLE_UPPER.lock();
    let allocator = &mut *BUDDY_ALLOCATOR.lock();
    let temporary_page = &mut TEMPORARY_PAGE.lock();
    // SAFETY: This is just a helper function, the options contract are uphold by the caller
    unsafe {
        upper.create_mappings(
            f,
            &mut TableManipulationContext { temporary_page, allocator, temporary_page_mapper: None },
            options,
        )
    }
}

/// Just a helper See [`ActivePageTable::with`] for more info
///
/// # Safety
/// See [`ActivePageTable::with`].
pub unsafe fn mapper_lower_with<R>(
    f: impl FnOnce(&mut Mapper<RecurseLevel4LowerHalf>, &mut BuddyAllocator) -> R,
    with: &mut InactivePageTable<RecurseLevel4LowerHalf>,
) -> R {
    let upper = &mut *ACTIVE_TABLE_UPPER.lock();
    let allocator = &mut *BUDDY_ALLOCATOR.lock();
    let temporary_page = &mut TEMPORARY_PAGE.lock();
    unsafe {
        upper.with(with, &mut TableManipulationContext { temporary_page, allocator, temporary_page_mapper: None }, f)
    }
}

pub fn mapper_lower<R>(f: impl FnOnce(&mut MapperWithAllocator<RecurseLevel4LowerHalf, BuddyAllocator>) -> R) -> R {
    let table = ACTIVE_TABLE_LOWER.inner_mut().get_mut();
    f(&mut table.mapper_with_allocator(&mut BUDDY_ALLOCATOR.lock()))
}

pub fn mapper_upper<R>(f: impl FnOnce(&mut MapperWithAllocator<RecurseLevel4UpperHalf, BuddyAllocator>) -> R) -> R {
    let mut table = ACTIVE_TABLE_UPPER.lock();
    f(&mut table.mapper_with_allocator(&mut BUDDY_ALLOCATOR.lock()))
}

pub fn init_local(ctx: &mut InitializationContext<Stage4>) {
    initialize_guard!();

    ctx.local_initializer(|i| {
        i.context_transformer(|builder, context| {
            let (table_lower, table_upper) = context.context.take_active_table().unwrap().split();
            builder
                .original_table(Arc::new(table_upper.into()))
                .bsp_only_table(Arc::new(Some(table_lower).into()))
                .stack_allocator(Arc::new(context.context.take_stack_allocator().unwrap().into()))
                .buddy_allocator(Arc::new(context.context.take_buddy_allocator().unwrap().into()))
                .temporary_page(Arc::new(context.context.take_temporary_page().unwrap().into()));
        });
        i.register(|builder, context, id| {
            local_builder!(
                builder,
                ACTIVE_TABLE_UPPER(Arc::clone(&context.original_table)),
                STACK_ALLOCATOR(Arc::clone(&context.stack_allocator)),
                BUDDY_ALLOCATOR(Arc::clone(&context.buddy_allocator)),
                TEMPORARY_PAGE(Arc::clone(&context.temporary_page)),
            );

            if id.is_bsp() {
                let bsp_only = context.bsp_only_table.lock().take().expect("BSP lower half table has been stolen");
                local_builder!(builder, ACTIVE_TABLE_LOWER(RefCell::new(bsp_only)));
                return;
            }

            let mut table_upper = context.original_table.lock();
            let mut table_manipulation_context = TableManipulationContext {
                temporary_page: &mut context.temporary_page.lock(),
                allocator: &mut *context.buddy_allocator.lock(),
                temporary_page_mapper: None,
            };
            // SAFETY: Since the ACTIVE_TABLE_UPPER is locked behind a shared mutex, the safety
            // contract upholds
            let new_table = unsafe {
                table_upper.create_mappings::<_, _, RecurseLevel4>(
                    |_, _| {},
                    &mut table_manipulation_context,
                    InactivePageCopyOption::upper_half(),
                )
            };

            // SAFETY: The new table upper half is copied from the currently active page table we
            // didn't modify anything
            let (_, active_lower) = unsafe { table_upper.switch_split(new_table) };

            local_builder!(builder, ACTIVE_TABLE_LOWER(active_lower.into()));
        })
    });
}

/// Initialize the memory
///
/// If this is being called outside kernel initialization this will panic
pub fn init(mut ctx: InitializationContext<Stage0>) -> InitializationContext<Stage1> {
    initialize_guard!();
    // SAFETY: This safe because the initialize_guard_above
    unsafe { prepare_flags() };

    // SAFETY: This safe because the initialize_guard_above
    let mut allocator = unsafe { init_allocator(&ctx) };
    let active_table = unsafe { remap_the_kernel(&mut allocator, &mut ctx) };

    DWARF_DATA.init_once(|| ctx.context_mut().boot_bridge.dwarf_baker());

    log!(
        Info,
        "Usable memory: {:.2} GB",
        allocator.max_mem() as f32 / (1 << 30) as f32 // TO GB
    );

    let mut ctx = ctx.next((
        active_table,
        allocator,
        StackAllocator::new(
            {
                let stack_alloc_start = virt_addr_alloc(STACK_ALLOC_SIZE);
                let stack_alloc_end = stack_alloc_start.start_address().as_u64() + STACK_ALLOC_SIZE * PAGE_SIZE;
                Page::range_inclusive(stack_alloc_start, VirtAddr::new(stack_alloc_end).into())
            },
            false,
        ),
        TemporaryPage::new(),
    ));

    unsafe {
        // SAFETY: This is called after the memory controller is initialize above
        allocator::init(&mut ctx);
    }

    ctx
}

/// Prepare the processor flags
/// e.g No-execute Write-protected
///
/// # Safety
/// The caller must ensure that this is only called on kernel initialization
pub unsafe fn prepare_flags() {
    let esi = CpuId::new().get_extended_state_info().unwrap();
    log!(Debug, "Support AVX256?: {}", esi.xcr0_supports_avx_256());
    log!(Debug, "Support AVX512 High?: {}", esi.xcr0_supports_avx512_zmm_hi256());
    log!(Debug, "Support AVX512 High Regs?: {}", esi.xcr0_supports_avx512_zmm_hi16());
    unsafe {
        enable_nxe_bit();
        enable_write_protect_bit();

        Cr4::write_or(Cr4Flags::OSXSAVE | Cr4Flags::OSFXS);
        let mut flags = Xcr0::empty();
        if esi.xcr0_supports_sse_128() {
            flags |= Xcr0::SEE;
        }
        if esi.xcr0_supports_avx_256() {
            flags |= Xcr0::AVX;
        }
        if esi.xcr0_supports_avx512_zmm_hi256() {
            flags |= Xcr0::ZMM_HIGH256;
        }
        if esi.xcr0_supports_avx512_zmm_hi16() {
            flags |= Xcr0::HI16_ZMM;
        }
        flags.write_retained();
    }
}

/// Initialize the buddy allocator and the kernel stack
///
/// # Safety
/// The caller must ensure that this is only called on kernel initialization
/// and the bootbridge memory map is valid
unsafe fn init_allocator(ctx: &InitializationContext<Stage0>) -> BuddyAllocator<64> {
    let area_allocator = unsafe { AreaAllocator::new(ctx.context().boot_bridge().memory_map()) };
    log!(Info, "UEFI memory map usable:");
    ctx.context().boot_bridge().memory_map().entries().filter(|e| e.ty == MemoryType::CONVENTIONAL).for_each(
        |descriptor| {
            log!(
                Info,
                "Range: Phys: [{:#016x}-{:#016x}]",
                descriptor.phys_start,
                descriptor.phys_start + descriptor.page_count * PAGE_SIZE,
            );
        },
    );
    unsafe { BuddyAllocator::new(area_allocator) }
}

unsafe fn enable_write_protect_bit() {
    unsafe { Cr0::WriteProtect.write_retained() };
}

unsafe fn enable_nxe_bit() {
    unsafe { Efer::NoExecuteEnable.write_retained() };
}

pub struct WithMapper<'a, T, A: FrameAllocator, P4: TopLevelP4> {
    table: &'a mut Mapper<P4>,
    with_table: &'a mut T,
    allocator: &'a mut A,
}

impl<'a, T, A: FrameAllocator, P4: TopLevelP4> WithMapper<'a, T, A, P4> {
    pub fn new(active_table: &'a mut ActivePageTable<P4>, with_table: &'a mut T, allocator: &'a mut A) -> Self {
        Self { table: active_table, with_table, allocator }
    }
}

pub struct MMIOBuffer {
    start: VirtAddr,
    size_in_pages: usize,
}

#[derive(Clone)]
pub struct MMIOBufferInfo {
    addr: PhysAddr,
    size_in_pages: usize,
}

impl MMIOBufferInfo {
    /// Create a new buffer info
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provide address and size is valid
    pub unsafe fn new_raw(addr: PhysAddr, size_in_pages: usize) -> Self {
        Self { addr, size_in_pages }
    }
}

impl From<RawData> for MMIOBufferInfo {
    fn from(value: RawData) -> Self {
        // SAFETY: We know this is safe because the raw data is only created at bootloader time
        unsafe { Self::new_raw(value.start(), value.size() / PAGE_SIZE as usize + 1) }
    }
}

impl MMIOBuffer {
    pub fn base(&self) -> VirtAddr {
        self.start
    }

    pub fn as_slice<T>(self) -> &'static mut [T] {
        // SAFETY: This is safe because the mmio is gurentee to be valid
        unsafe {
            core::slice::from_raw_parts_mut(
                self.start.as_mut_ptr::<T>(),
                self.size_in_pages * PAGE_SIZE as usize / size_of::<T>(),
            )
        }
    }
}

pub trait MMIODevice<Args> {
    fn boot_bridge(_bootbridge: &BootBridge) -> Option<MMIOBufferInfo> {
        None
    }

    fn acpi(_acpi: &Acpi) -> Option<MMIOBufferInfo> {
        None
    }

    fn other() -> Option<MMIOBufferInfo> {
        None
    }

    fn new(buffer: MMIOBuffer, args: Args) -> Self;
}

impl MMIOBufferInfo {
    pub fn size_in_pages(&self) -> usize {
        self.size_in_pages
    }

    pub fn size_in_bytes(&self) -> usize {
        self.size_in_pages * PAGE_SIZE as usize
    }

    pub fn addr(&self) -> PhysAddr {
        self.addr
    }
}

select_context! {
    (Stage2, Stage3, Stage4) => {
        pub fn mmio_device<T: MMIODevice<A>, A>(
            &mut self,
            args: A,
            depends: Option<MMIOBufferInfo>,
        ) -> Option<T> {
            let info = T::boot_bridge(&self.context().boot_bridge)
                .or(T::acpi(self.context().acpi()))
                .or(T::other())
                .or(depends)?;
            let vaddr = virt_addr_alloc(info.size_in_pages() as u64);
            let ctx = self.context_mut();
            // SAFETY: We know that the MMIOBufferInfo gurentee to be valid
            unsafe {
                ctx.active_table.map_to_range(
                    Page::containing_address(vaddr.start_address()),
                    Page::containing_address(vaddr.start_address() + info.size_in_bytes() as u64 - 1),
                    Frame::containing_address(info.addr()),
                    Frame::containing_address(info.addr() + info.size_in_bytes() - 1),
                    EntryFlags::WRITABLE | EntryFlags::NO_CACHE | EntryFlags::NO_EXECUTE,
                    &mut ctx.buddy_allocator,
                )
            };
            let buf = MMIOBuffer {
                start: vaddr.start_address().align_to(info.addr()),
                size_in_pages: info.size_in_pages(),
            };
            Some(T::new(buf, args))
        }
    }
    (Stage1, Stage2, Stage3, Stage4) => {
        pub fn stack_allocator(&mut self) -> WithMapper<'_, StackAllocator, BuddyAllocator<64>, RecurseLevel4> {
            let ctx = self.context_mut();
            ctx.stack_allocator.with_table(&mut ctx.active_table, &mut ctx.buddy_allocator)
        }

        /// Access the mapping of the InactivePageTable.
        ///
        /// # Safety
        /// See [`ActivePageTable::with`] safety docs
        pub unsafe fn with_inactive(&mut self, table: &mut InactivePageTable<RecurseLevel4>, f: impl FnOnce(&mut Mapper<RecurseLevel4>, &mut BuddyAllocator<64>)) {
            let ctx = self.context_mut();
            unsafe {
                ctx.active_table.with(table, &mut TableManipulationContext {
                    temporary_page: &mut ctx.temporary_page,
                    allocator: &mut ctx.buddy_allocator,
                    temporary_page_mapper: None
                }, f)
            }
        }

        pub fn buddy_allocator(&mut self) -> &mut BuddyAllocator<64> {
            let ctx = self.context_mut();
            &mut ctx.buddy_allocator
        }

        pub fn mapper<'a>(&'a mut self) -> MapperWithAllocator<'a, RecurseLevel4, BuddyAllocator<64>> {
            let ctx = self.context_mut();
            ctx.active_table
                .mapper_with_allocator(&mut ctx.buddy_allocator)
        }

        pub fn map(&mut self, size: usize, flags: EntryFlags) -> Page {
            let ctx = self.context_mut();
            let start_page = virt_addr_alloc(size as u64 / PAGE_SIZE + 1);
            ctx.active_table.map_range(
                start_page,
                Page::containing_address(start_page.start_address() + size - 1),
                flags,
                &mut ctx.buddy_allocator,
            );

            start_page
        }
    }
}
