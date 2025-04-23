use core::sync::atomic::{AtomicU64, Ordering};

use allocator::{
    area_allocator::AreaAllocator, buddy_allocator::BuddyAllocator,
    linear_allocator::LinearAllocator,
};
use bootbridge::{BootBridge, MemoryType, RawData};
use pager::{
    address::{Frame, Page, PhysAddr, VirtAddr},
    registers::{Cr0, Cr0Flags, Efer, EferFlags},
    EntryFlags, PAGE_SIZE,
};
use paging::{
    early_map_kernel, mapper::MapperWithAllocator, table::RecurseLevel4, ActivePageTable,
};
use stack_allocator::StackAllocator;

use crate::{
    driver::acpi::Acpi,
    initialization_context::{InitializationContext, Phase0, Phase1, Phase2, Phase3},
    initialize_guard, log,
};

pub use self::paging::remap_the_kernel;

pub mod allocator;
pub mod paging;
pub mod stack_allocator;

pub const MAX_ALIGN: usize = 8192;

/// Initialize the memory
///
/// If this is being called outside kernel initialization this will panic
pub fn init(ctx: InitializationContext<Phase0>) -> InitializationContext<Phase1> {
    initialize_guard!();
    // SAFETY: This safe because the initialize_guard_above
    unsafe { prepare_flags() };

    // SAFETY: This safe because the initialize_guard_above
    let (mut allocator, stack_allocator) = unsafe { init_allocator(&ctx) };
    let active_table = unsafe { remap_the_kernel(&mut allocator, &stack_allocator, &ctx) };

    log!(
        Info,
        "Usable memory: {:.2} GB",
        allocator.max_mem() as f32 / (1 << 30) as f32 // TO GB
    );

    let mut ctx = ctx.next((active_table, allocator, stack_allocator));

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
    unsafe {
        enable_nxe_bit();
        enable_write_protect_bit();
    }
}

/// Initialize the buddy allocator and the kernel stack
///
/// # Safety
/// The caller must ensure that this is only called on kernel initialization
/// and the bootbridge memory map is valid
unsafe fn init_allocator(
    ctx: &InitializationContext<Phase0>,
) -> (BuddyAllocator<64>, StackAllocator) {
    let mut area_allocator =
        unsafe { AreaAllocator::new(ctx.context().boot_bridge().memory_map()) };
    let buddy_allocator_allocator = area_allocator
        .linear_allocator(128)
        .expect("Not enough contiguous chunk of memory to boot the kernel");
    let kernel_stack_range = area_allocator
        .linear_allocator(512)
        .expect("Failed to allocate stack for the kernel");
    log!(
        Trace,
        "Buddy allocator range: [{:#016x}-{:#016x}]",
        buddy_allocator_allocator.original_start(),
        buddy_allocator_allocator.end()
    );
    log!(
        Trace,
        "Kernel stack range: [{:#016x}-{:#016x}]",
        kernel_stack_range.original_start(),
        kernel_stack_range.end()
    );
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
    unsafe {
        early_map_kernel(ctx, &buddy_allocator_allocator);
    }
    let stack_alloc = unsafe { StackAllocator::new(kernel_stack_range.range_page()) };
    (
        unsafe {
            BuddyAllocator::new(buddy_allocator_allocator, area_allocator, &stack_alloc, ctx)
        },
        stack_alloc,
    )
}

unsafe fn enable_write_protect_bit() {
    unsafe { Cr0::write_or(Cr0Flags::WriteProtect) };
}

unsafe fn enable_nxe_bit() {
    unsafe { Efer::write_or(EferFlags::NoExecuteEnable) };
}

const VIRT_BASE_ADDR: u64 = 0xFFFFFFFF00000000;
static CURRENT_ADDR: AtomicU64 = AtomicU64::new(VIRT_BASE_ADDR);

pub fn virt_addr_alloc(size_in_pages: u64) -> Page {
    let page = Page::containing_address(VirtAddr::new(
        CURRENT_ADDR.fetch_add(size_in_pages * PAGE_SIZE, Ordering::SeqCst),
    ));
    page
}

pub struct WithTable<'a, T> {
    table: &'a mut ActivePageTable<RecurseLevel4>,
    with_table: &'a mut T,
}

impl<'a, T> WithTable<'a, T> {
    pub fn new(
        active_table: &'a mut ActivePageTable<RecurseLevel4>,
        with_table: &'a mut T,
    ) -> Self {
        Self {
            table: active_table,
            with_table,
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
        unsafe { Self::new_raw(value.start(), value.size() / PAGE_SIZE as usize) }
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

    fn acpi(acpi: &Acpi) -> Option<MMIOBufferInfo> {
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

impl InitializationContext<Phase3> {
    pub fn mapper<'a>(&'a mut self) -> MapperWithAllocator<'a, RecurseLevel4, BuddyAllocator<64>> {
        let ctx = self.context_mut();
        ctx.active_table
            .mapper_with_allocator(&mut ctx.buddy_allocator)
    }

    pub fn stack_allocator(&mut self) -> WithTable<StackAllocator> {
        let ctx = self.context_mut();
        ctx.stack_allocator.with_table(&mut ctx.active_table)
    }

    pub fn mmio_device<T: MMIODevice<A>, A>(
        &mut self,
        args: A,
        depends: Option<MMIOBufferInfo>,
    ) -> Option<T> {
        let info = T::boot_bridge(&self.context().boot_bridge)
            .or_else(|| T::acpi(self.context().acpi()))
            .or_else(|| T::other())
            .or_else(|| depends)?;
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
            start: vaddr.start_address(),
            size_in_pages: info.size_in_pages(),
        };
        Some(T::new(buf, args))
    }
}

impl InitializationContext<Phase2> {
    pub fn mmio_device<T: MMIODevice<A>, A>(
        &mut self,
        args: A,
        depends: Option<MMIOBufferInfo>,
    ) -> Option<T> {
        let info = T::boot_bridge(&self.context().boot_bridge)
            .or_else(|| T::acpi(self.context().acpi()))
            .or_else(|| T::other())
            .or_else(|| depends)?;
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
            start: vaddr.start_address(),
            size_in_pages: info.size_in_pages(),
        };
        Some(T::new(buf, args))
    }

    pub fn mapper<'a>(&'a mut self) -> MapperWithAllocator<'a, RecurseLevel4, BuddyAllocator<64>> {
        let ctx = self.context_mut();
        ctx.active_table
            .mapper_with_allocator(&mut ctx.buddy_allocator)
    }

    pub fn stack_allocator(&mut self) -> WithTable<StackAllocator> {
        let ctx = self.context_mut();
        ctx.stack_allocator.with_table(&mut ctx.active_table)
    }
}

impl InitializationContext<Phase1> {
    pub fn mapper<'a>(&'a mut self) -> MapperWithAllocator<'a, RecurseLevel4, BuddyAllocator<64>> {
        let ctx = self.context_mut();
        ctx.active_table
            .mapper_with_allocator(&mut ctx.buddy_allocator)
    }

    pub fn stack_allocator(&mut self) -> WithTable<StackAllocator> {
        let ctx = self.context_mut();
        ctx.stack_allocator.with_table(&mut ctx.active_table)
    }
}

pub trait FrameAllocator {
    fn linear_allocator(&mut self, size_in_frames: u64) -> Option<LinearAllocator> {
        let mut last_address = 0;
        let mut counter = size_in_frames;
        let mut start_frame = Frame::null();
        loop {
            let frame = match self.allocate_frame() {
                Some(frame) => frame,
                None => return None,
            };
            if start_frame.start_address().as_u64() == 0 {
                start_frame = frame.clone();
            }
            // If the memory is not contiguous, reset the counter
            if last_address + PAGE_SIZE != frame.start_address().as_u64() && last_address != 0 {
                counter = size_in_frames;
                start_frame = frame.clone();
            }
            last_address = frame.start_address().as_u64();
            counter -= 1;
            if counter == 0 {
                break;
            }
        }
        assert!(start_frame.start_address().as_u64() != 0);
        // We know that the frame allocator is valid
        Some(unsafe {
            LinearAllocator::new(
                start_frame.start_address(),
                (size_in_frames * PAGE_SIZE) as usize,
            )
        })
    }

    // SAFETY: The implementor of this function must gurentee that the return frame is valid and is
    // the only ownership of that physical frame
    fn allocate_frame(&mut self) -> Option<Frame>;

    fn deallocate_frame(&mut self, frame: Frame);
}
