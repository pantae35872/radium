#![no_std]

use core::{cell::OnceCell, fmt::Debug};

use bakery::DwarfBaker;
use c_enum::c_enum;
use pager::{
    address::{Frame, PhysAddr, VirtAddr},
    allocator::linear_allocator::LinearAllocator,
    DataBuffer, EntryFlags, IdentityMappable, VirtuallyReplaceable,
};
use santa::Elf;

#[derive(Debug, Clone, Copy)]
pub struct RawData {
    start: PhysAddr,
    size: usize,
}

#[derive(Debug)]
pub struct KernelConfig {
    pub font_pixel_size: usize,
    pub log_level: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct MemoryDescriptor {
    pub ty: MemoryType,
    pub phys_start: PhysAddr,
    pub virt_start: VirtAddr,
    pub page_count: u64,
    pub att: u64,
}

c_enum! {
pub enum MemoryType: u32 {
    RESERVED                = 0
    LOADER_CODE             = 1
    LOADER_DATA             = 2
    BOOT_SERVICES_CODE      = 3
    BOOT_SERVICES_DATA      = 4
    RUNTIME_SERVICES_CODE   = 5
    RUNTIME_SERVICES_DATA   = 6
    CONVENTIONAL            = 7
    UNUSABLE                = 8
    ACPI_RECLAIM            = 9
    ACPI_NON_VOLATILE       = 10
    MMIO                    = 11
    MMIO_PORT_SPACE         = 12
    PAL_CODE                = 13
    PERSISTENT_MEMORY       = 14
}
}

#[derive(Debug, Clone)]
pub struct MemoryMapIter<'buf> {
    memory_map: &'buf MemoryMap<'buf>,
    index: usize,
}

/// A reimplementation of the uefi memory map
#[derive(Debug)]
pub struct MemoryMap<'a> {
    memory_map: DataBuffer<'a>,
    entry_size: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum PixelFormat {
    Rgb,
    Bgr,
    Bitmask(PixelBitmask),
    BltOnly,
}

#[derive(Debug, Clone, Copy)]
pub struct PixelBitmask {
    pub red: u32,
    pub green: u32,
    pub blue: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GraphicsInfo {
    resolution: (usize, usize),
    stride: usize,
    pixel_format: PixelFormat,
}

#[derive(Debug)]
#[repr(C)]
pub struct RawBootBridge {
    framebuffer_data: RawData,
    font_data: RawData,
    dwarf_data: Option<DwarfBaker<'static>>,
    kernel_elf: Elf<'static>,
    kernel_config: KernelConfig,
    memory_map: MemoryMap<'static>,
    graphics_info: GraphicsInfo,
    rsdp: PhysAddr,
    kernel_base: PhysAddr,
    early_alloc: LinearAllocator,
}

pub struct BootBridgeBuilder<A>
where
    A: Fn(usize) -> *mut u8,
{
    // An allocator provided by the bootloader that should never fails
    allocator: A,
    boot_bridge: OnceCell<*mut RawBootBridge>,
}

pub struct BootBridge(*mut RawBootBridge);

impl BootBridge {
    pub fn new(ptr: *mut RawBootBridge) -> Self {
        BootBridge(ptr)
    }

    pub(crate) fn deref(&self) -> &'static RawBootBridge {
        unsafe { &*self.0 }
    }

    pub(crate) fn deref_mut(&mut self) -> &'static mut RawBootBridge {
        unsafe { &mut *self.0 }
    }

    pub fn rsdp(&self) -> PhysAddr {
        self.deref().rsdp
    }

    pub fn graphics_info(&self) -> GraphicsInfo {
        self.deref().graphics_info
    }

    pub fn memory_map(&self) -> &'static MemoryMap<'static> {
        &self.deref().memory_map
    }

    pub fn framebuffer_data(&self) -> RawData {
        self.deref().framebuffer_data
    }

    pub fn font_data(&self) -> RawData {
        self.deref().font_data
    }

    pub fn font_size(&self) -> usize {
        self.deref().kernel_config.font_pixel_size
    }

    pub fn kernel_base(&self) -> PhysAddr {
        self.deref().kernel_base
    }

    pub fn log_level(&self) -> u64 {
        self.deref().kernel_config.log_level
    }

    pub fn early_alloc(&self) -> &LinearAllocator {
        &self.deref().early_alloc
    }

    pub fn kernel_elf(&self) -> &Elf<'static> {
        &self.deref().kernel_elf
    }

    pub fn dwarf_baker(&mut self) -> DwarfBaker<'static> {
        self.deref_mut().dwarf_data.take().unwrap()
    }

    pub fn ptr(&self) -> usize {
        self.0 as usize
    }
}

impl<A> IdentityMappable for BootBridgeBuilder<A>
where
    A: Fn(usize) -> *mut u8,
{
    fn map(&self, mapper: &mut impl pager::Mapper) {
        let boot_bridge = *self.boot_bridge.get().unwrap();
        unsafe {
            mapper.identity_map_by_size(
                Frame::containing_address(PhysAddr::new(boot_bridge as u64)),
                size_of::<RawBootBridge>(),
                EntryFlags::WRITABLE,
            );
            (*boot_bridge).dwarf_data.as_ref().unwrap().map(mapper);
            (*boot_bridge).kernel_elf.map(mapper);
        };
    }
}

impl VirtuallyReplaceable for BootBridge {
    fn replace<T: pager::Mapper>(&mut self, mapper: &mut pager::MapperWithVirtualAllocator<T>) {
        let current = self.0;
        let new = unsafe {
            mapper.map(
                PhysAddr::new(current as u64),
                size_of::<RawBootBridge>(),
                EntryFlags::WRITABLE,
            )
        };
        self.deref_mut().memory_map.memory_map.replace(mapper);
        self.deref_mut().kernel_elf.replace(mapper);
        if let Some(dwarf) = self.deref_mut().dwarf_data.as_mut() {
            dwarf.replace(mapper);
        }
        *self = Self::new(new.as_mut_ptr())
    }
}

impl IdentityMappable for BootBridge {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        unsafe {
            mapper.identity_map_by_size(
                PhysAddr::new(self.0 as u64).into(),
                size_of::<RawBootBridge>(),
                EntryFlags::WRITABLE,
            );
        };
        self.deref().memory_map.map(mapper);
        self.deref().kernel_elf.map(mapper);
    }
}

impl IdentityMappable for MemoryMap<'_> {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        self.memory_map.map(mapper);
    }
}

impl IdentityMappable for RawData {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        unsafe {
            mapper.identity_map_by_size(self.start().into(), self.size(), EntryFlags::WRITABLE)
        };
    }
}

impl<A: Fn(usize) -> *mut u8> Debug for BootBridgeBuilder<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x?}", self.boot_bridge.get())
    }
}

impl<A> BootBridgeBuilder<A>
where
    A: Fn(usize) -> *mut u8,
{
    pub fn new(allocator: A) -> Self {
        BootBridgeBuilder {
            allocator,
            boot_bridge: OnceCell::new(),
        }
    }

    fn inner_bridge(&mut self) -> &'static mut RawBootBridge {
        let boot_bridge = self.boot_bridge.get_or_init(|| {
            let ptr = (self.allocator)(core::mem::size_of::<RawBootBridge>());
            ptr as *mut RawBootBridge
        });

        unsafe { &mut **boot_bridge }
    }

    pub fn framebuffer_data(&mut self, start: u64, size: usize) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.framebuffer_data = RawData {
            start: PhysAddr::new(start),
            size,
        };
        self
    }

    pub fn kernel_config(&mut self, config: KernelConfig) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.kernel_config = config;
        self
    }

    pub fn kernel_elf(&mut self, elf: Elf<'static>) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.kernel_elf = elf;
        self
    }

    pub fn early_alloc(&mut self, early_alloc: LinearAllocator) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.early_alloc = early_alloc;
        self
    }

    pub fn font_data(&mut self, start: u64, size: usize) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.font_data = RawData {
            start: PhysAddr::new(start),
            size,
        };
        self
    }

    pub fn dwarf_data(&mut self, dwarf: DwarfBaker<'static>) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.dwarf_data = Some(dwarf);
        self
    }

    pub fn graphics_info(
        &mut self,
        resolution: (usize, usize),
        stride: usize,
        pixel_format: PixelFormat,
    ) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.graphics_info = GraphicsInfo::new(resolution, stride, pixel_format);
        self
    }

    pub fn memory_map(&mut self, memory_map: &'static [u8], entry_size: usize) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.memory_map = MemoryMap::new(memory_map, entry_size);
        self
    }

    pub fn kernel_base(&mut self, base: PhysAddr) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.kernel_base = base;
        self
    }

    pub fn rsdp(&mut self, rsdp: u64) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.rsdp = PhysAddr::new(rsdp);
        self
    }

    /// Build the boot bridge and const return a pointer to it
    /// Failed if the boot bridge is not initialized
    pub fn build(self) -> Option<*mut RawBootBridge> {
        self.boot_bridge
            .get()
            .copied()
            .map(|e| e as *mut RawBootBridge)
    }
}

impl RawData {
    pub fn start(&self) -> PhysAddr {
        self.start
    }
    pub fn size(&self) -> usize {
        self.size
    }
    pub fn end(&self) -> PhysAddr {
        self.start + self.size - 1
    }
}

impl<'a> MemoryMap<'a> {
    pub fn new(memory_map: &'static [u8], entry_size: usize) -> Self {
        MemoryMap {
            memory_map: DataBuffer::new(memory_map),
            entry_size,
        }
    }

    pub fn get(&self, index: usize) -> Option<&'a MemoryDescriptor> {
        if index >= self.memory_map.len() / self.entry_size {
            return None;
        }
        let desc = unsafe {
            &*(self.memory_map.as_ptr().add(index * self.entry_size) as *const MemoryDescriptor)
        };
        Some(desc)
    }

    pub fn entries(&'a self) -> MemoryMapIter<'a> {
        MemoryMapIter {
            memory_map: self,
            index: 0,
        }
    }
}

impl GraphicsInfo {
    pub fn new(resolution: (usize, usize), stride: usize, pixel_format: PixelFormat) -> Self {
        GraphicsInfo {
            resolution,
            stride,
            pixel_format,
        }
    }

    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn pixel_format(&self) -> &PixelFormat {
        &self.pixel_format
    }

    pub fn resolution(&self) -> (usize, usize) {
        self.resolution
    }
}

impl<'a> Iterator for MemoryMapIter<'a> {
    type Item = &'a MemoryDescriptor;

    fn next(&mut self) -> Option<Self::Item> {
        let desc = self.memory_map.get(self.index)?;

        self.index += 1;

        Some(desc)
    }
}

impl Debug for BootBridge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let boot_bridge = self.deref();
        write!(
            f,
            "BootBridge {{ ptr: {:#x}, framebuffer_data: {:?}, font_data: {:?}, kernel_elf: {:?}, kernel_config: {:?}, rsdp: {} }}",
            self.0 as u64,
            boot_bridge.framebuffer_data,
            boot_bridge.font_data,
            boot_bridge.kernel_elf,
            boot_bridge.kernel_config,
            boot_bridge.rsdp.as_u64(),
        )
    }
}
