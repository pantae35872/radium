use core::sync::atomic::{AtomicU64, Ordering};

use allocator::{
    area_allocator::AreaAllocator, buddy_allocator::BuddyAllocator,
    linear_allocator::LinearAllocator,
};
use bootbridge::{BootBridge, MemoryType};
use conquer_once::spin::OnceCell;
use paging::{early_map_kernel, table::RecurseLevel4, ActivePageTable, EntryFlags, Page};
use spin::Mutex;
use stack_allocator::{Stack, StackAllocator};
use x86_64::{
    registers::control::{Cr0Flags, Cr3, EferFlags},
    PhysAddr, VirtAddr,
};

use crate::log;

pub use self::paging::remap_the_kernel;

pub mod allocator;
pub mod paging;
pub mod stack_allocator;

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_ALIGN: usize = 8192;

pub fn init(bootbridge: &BootBridge) {
    enable_nxe_bit();
    enable_write_protect_bit();

    let (mut allocator, stack_allocator) = init_allocator(bootbridge);
    let active_table = remap_the_kernel(&mut allocator, &stack_allocator, bootbridge);

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

fn init_allocator(bootbridge: &BootBridge) -> (BuddyAllocator<64>, StackAllocator) {
    let mut area_allocator = AreaAllocator::new(bootbridge.memory_map());
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
    bootbridge
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
        early_map_kernel(bootbridge, &buddy_allocator_allocator);
    }
    let stack_alloc = StackAllocator::new(Page::range_inclusive(
        Page::containing_address(kernel_stack_range.original_start() as u64),
        Page::containing_address(
            (kernel_stack_range.original_start() + kernel_stack_range.size() - 1) as u64,
        ),
    ));
    (
        unsafe {
            BuddyAllocator::new(
                bootbridge.kernel_elf(),
                buddy_allocator_allocator,
                area_allocator,
                &stack_alloc,
            )
        },
        stack_alloc,
    )
}

pub fn enable_write_protect_bit() {
    use x86_64::registers::control::Cr0;

    unsafe {
        let mut cr0 = Cr0::read();
        cr0.insert(Cr0Flags::WRITE_PROTECT);
        Cr0::write(cr0);
    }
}

pub fn enable_nxe_bit() {
    use x86_64::registers::model_specific::Efer;

    unsafe {
        let mut efer = Efer::read();
        efer.insert(EferFlags::NO_EXECUTE_ENABLE);
        Efer::write(efer);
    }
}

static MEMORY_CONTROLLER: OnceCell<Mutex<MemoryController<64>>> = OnceCell::uninit();

pub fn memory_controller() -> &'static Mutex<MemoryController<64>> {
    MEMORY_CONTROLLER
        .get()
        .expect("Memory controller not initialized")
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
                log!(
                    Trace,
                    "Allocating vaddr ranges: [{:#016x}-{:#016x}]",
                    addr,
                    addr + size
                );
                return addr;
            }
            Err(updated) => addr = updated,
        }
    }
}

pub struct MemoryController<const ORDER: usize> {
    active_table: ActivePageTable<RecurseLevel4>,
    allocator: BuddyAllocator<ORDER>,
    stack_allocator: StackAllocator,
}

pub trait MMIODevice {
    fn start(&self) -> Option<u64>;
    fn page_count(&self) -> Option<usize>;
    fn start_frame(&self) -> Option<Frame> {
        self.start().map(|e| Frame::containing_address(e))
    }
    fn end_frame(&self) -> Option<Frame> {
        self.start()
            .zip(self.page_count().map(|e| e as u64))
            .map(|(e, c)| Frame::containing_address(e + c * PAGE_SIZE - 1))
    }
    fn mapped(&mut self, vaddr: Option<u64>);
}

impl<const ORDER: usize> MemoryController<ORDER> {
    pub fn alloc_stack(&mut self, size_in_pages: usize) -> Option<Stack> {
        self.stack_allocator
            .alloc_stack(&mut self.active_table, size_in_pages)
    }

    fn map(&mut self, page: Page, flags: EntryFlags) {
        self.active_table.map(page, flags, &mut self.allocator);
    }

    pub fn alloc_map(&mut self, size: u64, start: u64) {
        let start_page = Page::containing_address(start);
        let end_page = Page::containing_address(start + size - 1);

        log!(
            Trace,
            "Allocate: [{:#016x}-{:#016x}], Actual Allocate (aligned): [{:#016x}-{:#016x}]",
            start,
            start + size - 1,
            start_page.start_address(),
            end_page.start_address() + PAGE_SIZE,
        );

        for page in Page::range_inclusive(start_page, end_page) {
            self.map(page, EntryFlags::WRITABLE | EntryFlags::PRESENT);
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
        log!(
            Trace,
            "Mapping: [{:#016x}-{:#016x}] to [{:#016x}-{:#016x}], Actual Map (aligned): [{:#016x}-{:#016x}] to [{:#016x}-{:#016x}], Flags: {}",
            virt_start,
            virt_start + size - 1,
            phy_start,
            phy_start + size - 1,
            start_page.start_address(),
            end_page.start_address() + PAGE_SIZE,
            start_frame.start_address(),
            end_frame.start_address() + PAGE_SIZE,
            flags
        );
        for (page, frame) in Page::range_inclusive(start_page, end_page)
            .zip(Frame::range_inclusive(start_frame, end_frame))
        {
            self.map_to(page, frame, flags);
        }
        return UnalignPhysicalMapGuard::new(phy_start);
    }

    /// If the mmio device may be mapped multiple times due to core initialization logic,
    /// the argument multiple_cores may be set to true
    pub fn map_mmio(&mut self, mmio_device: &mut impl MMIODevice, multiple_cores: bool) {
        let (start_frame, end_frame, page_count) = match (
            mmio_device.start_frame(),
            mmio_device.end_frame(),
            mmio_device.page_count(),
        ) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => {
                mmio_device.mapped(None);
                return;
            }
        };
        let virt_start = virt_addr_alloc(page_count as u64);
        let start_page = Page::containing_address(virt_start);
        let end_page = Page::containing_address(virt_start + page_count as u64 * PAGE_SIZE - 1);
        log!(
            Trace,
            "Mapping MMIO Device: [{:#016x}-{:#016x}] to [{:#016x}-{:#016x}]",
            start_page.start_address(),
            end_page.start_address() + PAGE_SIZE,
            start_frame.start_address(),
            end_frame.start_address() + PAGE_SIZE,
        );
        let mut flag = EntryFlags::PRESENT | EntryFlags::WRITABLE | EntryFlags::NO_CACHE;
        if multiple_cores {
            flag |= EntryFlags::OVERWRITEABLE
        }
        for (page, frame) in Page::range_inclusive(start_page, end_page)
            .zip(Frame::range_inclusive(start_frame, end_frame))
        {
            self.map_to(page, frame, flag);
        }
        mmio_device.mapped(Some(virt_start));
    }

    pub fn ident_map(&mut self, size: u64, phy_start: u64, flags: EntryFlags) {
        let start = Frame::containing_address(phy_start);
        let end = Frame::containing_address(phy_start + size - 1);
        log!(
            Trace,
            "Identity map: [{:#016x}-{:#016x}], Actual Identity map: [{:#016x}-{:#016x}], Flags: {}",
            phy_start,
            phy_start + size - 1,
            start.start_address(),
            end.start_address() + PAGE_SIZE,
            flags,
        );
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

    pub unsafe fn current_page_phys(&self) -> u64 {
        Cr3::read().0.start_address().as_u64()
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
    #[must_use]
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

impl From<u64> for Frame {
    fn from(value: u64) -> Self {
        Self::containing_address(value)
    }
}

pub trait FrameAllocator {
    fn linear_allocator(&mut self, size_in_frames: u64) -> Option<LinearAllocator> {
        let mut last_address = 0;
        let mut counter = size_in_frames;
        let mut start_frame = Frame::containing_address(0);
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
                start_frame.start_address().as_u64() as usize,
                (size_in_frames * PAGE_SIZE) as usize,
            )
        })
    }

    fn allocate_frame(&mut self) -> Option<Frame>;
    fn deallocate_frame(&mut self, frame: Frame);
}

#[derive(Clone)]
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
