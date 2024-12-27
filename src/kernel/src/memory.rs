use core::sync::atomic::{AtomicU64, Ordering};

use allocator::buddy_allocator::BuddyAllocator;
use common::boot::BootInformation;
use conquer_once::spin::OnceCell;
use paging::{ActivePageTable, EntryFlags, Page};
use spin::Mutex;
use stack_allocator::{Stack, StackAllocator};
use x86_64::{
    registers::control::{Cr0Flags, EferFlags},
    PhysAddr, VirtAddr,
};

use crate::log;

pub use self::paging::remap_the_kernel;

pub mod allocator;
pub mod paging;
pub mod stack_allocator;

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_ALIGN: usize = 8192;

pub fn init(boot_info: &'static BootInformation) {
    let mut allocator = unsafe { BuddyAllocator::new(boot_info.memory_map()) };
    enable_nxe_bit();
    enable_write_protect_bit();
    let active_table = remap_the_kernel(&mut allocator, &boot_info);

    let stack_allocator = {
        let stack_alloc_start = Page::containing_address(virt_addr_alloc(409600));
        let stack_alloc_end = stack_alloc_start + 100;
        let stack_alloc_range = Page::range_inclusive(stack_alloc_start, stack_alloc_end);
        StackAllocator::new(stack_alloc_range)
    };
    MEMORY_CONTROLLER.init_once(|| {
        MemoryController {
            active_table,
            allocator,
            stack_allocator,
        }
        .into()
    });
    allocator::init();
    log!(
        Info,
        "Usable memory: {:.2} GB",
        memory_controller().lock().max_mem() as f32 / (1 << 30) as f32 // TO GB
    );
}

fn enable_write_protect_bit() {
    use x86_64::registers::control::Cr0;

    unsafe {
        let mut cr0 = Cr0::read();
        cr0.insert(Cr0Flags::WRITE_PROTECT);
        Cr0::write(cr0);
    }
}

fn enable_nxe_bit() {
    use x86_64::registers::model_specific::Efer;

    unsafe {
        let mut efer = Efer::read();
        efer.insert(EferFlags::NO_EXECUTE_ENABLE);
        Efer::write(efer);
    }
}

/// Switch the current scope to use a page table that is identity-mapped (1:1) with physical memory.
///
/// This macro temporarily switches the page table in the current scope to one where virtual addresses
/// directly map to the corresponding physical addresses. The identity mapping bypasses the standard virtual
/// memory translation, giving direct access to physical memory for the duration of the current scope.
///
/// ## Important Notes:
/// - **Heap allocation causes *undefined behavior*:** While the heap allocator is technically still accessible,
///   any attempt to allocate memory or deallocating memory in this mode will lead to *undefined behavior*. Avoid any operations that
///   require dynamic memory allocation.
/// - **No printing:** Functions that rely on virtual memory, such as printing to the console, will not work in this mode.
/// - **Limited OS features:** Many OS features that depend on virtual memory translation will be unavailable
///   while this macro is active.
///
/// ## Usage:
/// When invoked, this macro alters the memory mapping of the current scope. Upon exiting the scope, the page table
/// is restored to its previous state. All code within the scope will operate with the identity-mapped memory.
///
/// ### Safety:
/// This macro should only be used when you fully understand the implications of bypassing
/// the memory protection mechanisms provided by virtual memory.
///
/// Example:
/// ```rust
/// direct_mapping!({
///     // All memory access in this block is identity-mapped
/// });
/// ```
///
/// **Warning:** Ensure that code in this scope does not rely on heap allocation or other
/// features that depend on virtual memory.
#[macro_export]
macro_rules! direct_mapping {
    ($body:block) => {
        extern "C" {
            static p4_table: u8;
        }

        let current_table;

        x86_64::instructions::interrupts::disable();
        unsafe {
            let mut active_table = {
                use $crate::memory::paging::ActivePageTable;

                ActivePageTable::new()
            };
            let old_table = {
                use $crate::memory::paging::InactivePageTable;
                use $crate::memory::Frame;

                InactivePageTable::from_raw_frame(Frame::containing_address(
                    &p4_table as *const u8 as u64,
                ))
            };
            current_table = active_table.switch(old_table);
        }
        $crate::defer!(unsafe {
            let mut active_table = {
                use $crate::memory::paging::ActivePageTable;

                ActivePageTable::new()
            };
            active_table.switch(current_table);
            x86_64::instructions::interrupts::enable();
        });
        $body
    };
}

static MEMORY_CONTROLLER: OnceCell<Mutex<MemoryController<64>>> = OnceCell::uninit();

pub fn memory_controller() -> &'static Mutex<MemoryController<64>> {
    return MEMORY_CONTROLLER
        .get()
        .expect("Memory controller not initialized");
}

const VIRT_BASE_ADDR: u64 = 0xFFFFFFFF00000000;
const PAGE_ALIGN: u64 = 4096;
static CURRENT_ADDR: AtomicU64 = AtomicU64::new(VIRT_BASE_ADDR);

pub fn virt_addr_alloc(size: u64) -> u64 {
    let mut addr = CURRENT_ADDR.load(Ordering::Acquire);
    let mut new_addr;
    loop {
        new_addr = addr + size + (size as *const u8).align_offset(PAGE_ALIGN as usize) as u64;
        match CURRENT_ADDR.compare_exchange_weak(
            addr,
            new_addr,
            Ordering::Release,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                return addr;
            }
            Err(updated) => addr = updated,
        }
    }
}

pub struct MemoryController<const ORDER: usize> {
    active_table: ActivePageTable,
    allocator: BuddyAllocator<'static, ORDER>,
    stack_allocator: StackAllocator,
}

impl<const ORDER: usize> MemoryController<ORDER> {
    pub fn alloc_stack(&mut self, size_in_pages: usize) -> Option<Stack> {
        self.stack_allocator
            .alloc_stack(&mut self.active_table, &mut self.allocator, size_in_pages)
    }

    fn map(&mut self, page: Page, flags: EntryFlags) {
        self.active_table.map(page, flags, &mut self.allocator);
    }

    pub fn alloc_map(&mut self, size: u64, start: u64) {
        let start_page = Page::containing_address(start);
        let end_page = Page::containing_address(start + size - 1);

        for page in Page::range_inclusive(start_page, end_page) {
            self.map(
                page,
                EntryFlags::WRITABLE
                    | EntryFlags::PRESENT
                    | EntryFlags::WRITE_THROUGH
                    | EntryFlags::NO_CACHE,
            );
        }
    }

    /// Map the provided virtual address to the provided physical address. if the physical address
    /// is not align, will return a offset that used to offset the provided virtual address to match the provided physical address.
    pub fn phy_map(
        &mut self,
        size: u64,
        phy_start: u64,
        virt_start: u64,
        flags: EntryFlags,
    ) -> UnalignPhysicalMapGuard {
        let start_page = Page::containing_address(virt_start);
        let start_frame = Frame::containing_address(phy_start);
        let end_page = Page::containing_address(virt_start + size - 1);
        let end_frame = Frame::containing_address(phy_start + size - 1);
        for (page, frame) in Page::range_inclusive(start_page, end_page)
            .zip(Frame::range_inclusive(start_frame, end_frame))
        {
            self.map_to(page, frame, flags);
        }
        return UnalignPhysicalMapGuard::new(phy_start);
    }

    pub fn ident_map(&mut self, size: u64, phy_start: u64, flags: EntryFlags) {
        let start = Frame::containing_address(phy_start);
        let end = Frame::containing_address(phy_start + size - 1);
        Frame::range_inclusive(start, end).for_each(|frame| {
            self.active_table
                .identity_map(frame, flags, &mut self.allocator)
        });
    }

    pub fn unmap_addr(&mut self, mapped_start: u64, size: u64) {
        let start = Page::containing_address(mapped_start);
        let end = Page::containing_address(mapped_start + size - 1);
        Page::range_inclusive(start, end).for_each(|page| {
            self.active_table.unmap_addr(page);
        });
    }

    fn map_to(&mut self, page: Page, frame: Frame, flags: EntryFlags) {
        self.active_table
            .map_to(page, frame, flags, &mut self.allocator);
    }

    pub fn physical_alloc(&mut self, size: usize) -> Option<PhysAddr> {
        return self
            .allocator
            .allocate(size)
            .map(|ptr| PhysAddr::new(ptr as u64));
    }

    pub fn physical_dealloc(&mut self, addr: PhysAddr, size: usize) {
        self.allocator.dealloc(addr.as_u64() as *mut u8, size);
    }

    pub fn get_physical(&mut self, addr: VirtAddr) -> Option<PhysAddr> {
        return self.active_table.translate(addr);
    }

    pub fn max_mem(&self) -> usize {
        self.allocator.max_mem()
    }

    pub fn allocated(&self) -> usize {
        self.allocator.allocated()
    }
}

/// A guard for unalign physical map.
/// If the caller of phy_map not adding the offset correctly, this will issue a warning.
pub struct UnalignPhysicalMapGuard {
    offset: u64,
    used: bool,
}

impl UnalignPhysicalMapGuard {
    pub fn new(phy_addr: u64) -> Self {
        if (phy_addr as *const u8).is_aligned_to(PAGE_ALIGN as usize) {
            return Self::new_empty();
        }
        Self {
            offset: PAGE_ALIGN - (phy_addr as *const u8).align_offset(PAGE_ALIGN as usize) as u64,
            used: false,
        }
    }

    pub fn new_empty() -> Self {
        Self {
            offset: 0,
            used: true,
        }
    }

    /// Apply the provided virtual address to the required offset, consuming this in the process.
    pub fn apply(mut self, virt_addr: u64) -> u64 {
        self.used = true;
        virt_addr + self.offset
    }
}

impl Drop for UnalignPhysicalMapGuard {
    fn drop(&mut self) {
        if !self.used {
            log!(Warning, "Unused physical alignment for virtual address ");
        }
    }
}

#[derive(PartialEq, PartialOrd, Clone)]
pub struct Frame {
    number: u64,
}

impl Frame {
    pub fn containing_address(address: u64) -> Frame {
        Frame {
            number: address / PAGE_SIZE,
        }
    }
    pub fn start_address(&self) -> PhysAddr {
        PhysAddr::new(self.number * PAGE_SIZE)
    }
    pub fn range_inclusive(start: Frame, end: Frame) -> FrameIter {
        FrameIter { start, end }
    }
}

pub trait FrameAllocator {
    fn allocate_frame(&mut self) -> Option<Frame>;
    fn deallocate_frame(&mut self, frame: Frame);
}

pub struct FrameIter {
    start: Frame,
    end: Frame,
}

impl Iterator for FrameIter {
    type Item = Frame;

    fn next(&mut self) -> Option<Frame> {
        if self.start <= self.end {
            let frame = self.start.clone();
            self.start.number += 1;
            Some(frame)
        } else {
            None
        }
    }
}
