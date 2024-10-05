use x86_64::PhysAddr;

pub use self::paging::remap_the_kernel;

pub mod allocator;
pub mod paging;
pub mod stack_allocator;

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_ALIGN: usize = 8192;

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
