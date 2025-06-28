use allocator::{area_allocator::AreaAllocator, buddy_allocator::BuddyAllocator};
use bootbridge::{BootBridge, MemoryType, RawData};
use pager::{
    address::{Frame, Page, PhysAddr, VirtAddr},
    allocator::{virt_allocator::VirtualAllocator, FrameAllocator},
    paging::{mapper::MapperWithAllocator, table::RecurseLevel4, ActivePageTable},
    registers::{Cr0, Cr0Flags, Cr4, Cr4Flags, Efer, EferFlags, Xcr0, Xcr0Flags},
    EntryFlags, VirtuallyMappable, KERNEL_GENERAL_USE, PAGE_SIZE,
};
use raw_cpuid::CpuId;
use stack_allocator::StackAllocator;

use crate::{
    driver::acpi::Acpi,
    initialization_context::{select_context, InitializationContext, Stage0, Stage1},
    initialize_guard, log, DWARF_DATA,
};

pub use self::paging::remap_the_kernel;

pub mod allocator;
pub mod paging;
pub mod stack_allocator;

pub const MAX_ALIGN: usize = 8192;
pub const STACK_ALLOC_SIZE: u64 = 32768;

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
        StackAllocator::new({
            let stack_alloc_start = virt_addr_alloc(STACK_ALLOC_SIZE);
            let stack_alloc_end =
                stack_alloc_start.start_address().as_u64() + STACK_ALLOC_SIZE * PAGE_SIZE;
            Page::range_inclusive(stack_alloc_start, VirtAddr::new(stack_alloc_end).into())
        }),
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
    log!(
        Debug,
        "Support AVX512 High?: {}",
        esi.xcr0_supports_avx512_zmm_hi256()
    );
    log!(
        Debug,
        "Support AVX512 High Regs?: {}",
        esi.xcr0_supports_avx512_zmm_hi16()
    );
    unsafe {
        enable_nxe_bit();
        enable_write_protect_bit();

        Cr4::write_or(Cr4Flags::OSXSAVE | Cr4Flags::OSFXS);
        let mut flags = Xcr0Flags::empty();
        if esi.xcr0_supports_sse_128() {
            flags |= Xcr0Flags::SEE;
        }
        if esi.xcr0_supports_avx_256() {
            flags |= Xcr0Flags::AVX;
        }
        if esi.xcr0_supports_avx512_zmm_hi256() {
            flags |= Xcr0Flags::ZMM_HIGH256;
        }
        if esi.xcr0_supports_avx512_zmm_hi16() {
            flags |= Xcr0Flags::HI16_ZMM;
        }
        Xcr0::write_or(flags);
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
    ctx.context()
        .boot_bridge()
        .memory_map()
        .entries()
        .filter(|e| e.ty == MemoryType::CONVENTIONAL)
        .for_each(|descriptor| {
            log!(
                Info,
                "Range: Phys: [{:#016x}-{:#016x}]",
                descriptor.phys_start,
                descriptor.phys_start + descriptor.page_count * PAGE_SIZE,
            );
        });
    unsafe { BuddyAllocator::new(area_allocator) }
}

unsafe fn enable_write_protect_bit() {
    unsafe { Cr0::write_or(Cr0Flags::WriteProtect) };
}

unsafe fn enable_nxe_bit() {
    unsafe { Efer::write_or(EferFlags::NoExecuteEnable) };
}

static GENERAL_VIRTUAL_ALLOCATOR: VirtualAllocator = VirtualAllocator::new(
    KERNEL_GENERAL_USE,
    (VirtAddr::new(0xFFFF_F000_0000_0000).as_u64() - KERNEL_GENERAL_USE.as_u64()) as usize,
);

pub fn virt_addr_alloc(size_in_pages: u64) -> Page {
    GENERAL_VIRTUAL_ALLOCATOR
        .allocate(size_in_pages as usize)
        .expect("RAN OUT OF VIRTUAL ADDR")
}

pub struct WithTable<'a, T, A: FrameAllocator> {
    table: &'a mut ActivePageTable<RecurseLevel4>,
    with_table: &'a mut T,
    allocator: &'a mut A,
}

impl<'a, T, A: FrameAllocator> WithTable<'a, T, A> {
    pub fn new(
        active_table: &'a mut ActivePageTable<RecurseLevel4>,
        with_table: &'a mut T,
        allocator: &'a mut A,
    ) -> Self {
        Self {
            table: active_table,
            with_table,
            allocator,
        }
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
        Self {
            addr,
            size_in_pages,
        }
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
    (Stage2, Stage3, End) => {
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
    (Stage1, Stage2, Stage3, End) => {
        pub fn stack_allocator(&mut self) -> WithTable<'_, StackAllocator, BuddyAllocator<64>> {
            let ctx = self.context_mut();
            ctx.stack_allocator.with_table(&mut ctx.active_table, &mut ctx.buddy_allocator)
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

        pub fn virtually_map(&mut self, obj: &impl VirtuallyMappable, virt_base: VirtAddr, phys_base: PhysAddr) {
            let ctx = self.context_mut();
            ctx.active_table
                .virtually_map_object(obj, virt_base, phys_base, &mut ctx.buddy_allocator);
        }
    }
}
