use allocator::buddy_allocator::BuddyAllocator;
use common::boot::BootInformation;
use conquer_once::spin::OnceCell;
use paging::{ActivePageTable, EntryFlags, Page};
use proc::comptime_alloc;
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
        let stack_alloc_start = Page::containing_address(comptime_alloc!(409600));
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

        {
            let current_table;

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
            });
            $body
        }
    };
}

static MEMORY_CONTROLLER: OnceCell<Mutex<MemoryController<64>>> = OnceCell::uninit();

pub fn memory_controller() -> &'static Mutex<MemoryController<64>> {
    return MEMORY_CONTROLLER
        .get()
        .expect("Memory controller not initialized");
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

    pub fn phy_map(&mut self, size: u64, phy_start: u64, virt_start: u64) {
        let start_page = Page::containing_address(virt_start);
        let start_frame = Frame::containing_address(phy_start);
        let end_page = Page::containing_address(virt_start + size - 1);
        let end_frame = Frame::containing_address(phy_start + size - 1);
        for (page, frame) in Page::range_inclusive(start_page, end_page)
            .zip(Frame::range_inclusive(start_frame, end_frame))
        {
            self.map_to(
                page,
                frame,
                EntryFlags::PRESENT
                    | EntryFlags::NO_CACHE
                    | EntryFlags::WRITABLE
                    | EntryFlags::WRITE_THROUGH,
            );
        }
    }

    pub fn ident_map(&mut self, size: u64, phy_start: u64) {
        let start = Frame::containing_address(phy_start);
        let end = Frame::containing_address(phy_start + size - 1);
        Frame::range_inclusive(start, end).for_each(|frame| {
            self.active_table.identity_map(
                frame,
                EntryFlags::WRITABLE | EntryFlags::NO_CACHE | EntryFlags::PRESENT,
                &mut self.allocator,
            )
        });
    }

    fn map_to(&mut self, page: Page, frame: Frame, flags: EntryFlags) {
        self.active_table
            .map_to(page, frame, flags, &mut self.allocator);
    }

    pub fn allocate(&mut self, size: usize) -> Option<*mut u8> {
        return self.allocator.allocate(size);
    }

    pub fn deallocate(&mut self, ptr: *mut u8, size: usize) {
        self.allocator.dealloc(ptr, size);
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
