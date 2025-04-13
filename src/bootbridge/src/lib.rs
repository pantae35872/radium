#![no_std]

use core::{cell::OnceCell, fmt::Debug};

use c_enum::c_enum;
use santa::{Elf, PAGE_SIZE};

#[derive(Debug, Clone, Copy)]
pub struct RawData {
    start: u64,
    size: usize,
}

#[derive(Debug)]
pub struct KernelConfig {
    pub font_pixel_size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct MemoryDescriptor {
    pub ty: MemoryType,
    pub phys_start: u64,
    pub virt_start: u64,
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
    memory_map: &'a [u8],
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
    mem_capacity: u64,
    framebuffer_data: RawData,
    font_data: RawData,
    kernel_elf: Elf<'static>,
    kernel_config: KernelConfig,
    memory_map: MemoryMap<'static>,
    graphics_info: GraphicsInfo,
    rsdp: u64,
}

#[derive(Debug)]
pub struct BootBridgeBuilder<A>
where
    A: Fn(usize) -> *mut u8,
{
    // An allocator provided by the bootloader that should never fails
    allocator: A,
    boot_bridge: OnceCell<*mut RawBootBridge>,
}

pub struct BootBridge(pub *const RawBootBridge);

impl BootBridge {
    pub fn new(ptr: *const RawBootBridge) -> Self {
        BootBridge(ptr)
    }

    pub(crate) fn deref(&self) -> &'static RawBootBridge {
        unsafe { &*self.0 }
    }

    pub fn rsdp(&self) -> u64 {
        self.deref().rsdp
    }

    pub fn mem_capacity(&self) -> u64 {
        self.deref().mem_capacity
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

    pub fn kernel_elf(&self) -> &Elf<'static> {
        &self.deref().kernel_elf
    }

    pub fn map_self(&self, mut mapper: impl FnMut(u64, u64)) {
        let bridge_start = self.0 as *const u8 as u64;
        mapper(bridge_start, core::mem::size_of::<RawBootBridge>() as u64);

        let mem_map = self.deref().memory_map.memory_map;
        let map_start = mem_map as *const [u8] as *const u8 as u64;
        mapper(map_start, mem_map.len() as u64);

        self.kernel_elf().map_buffer(mapper);
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
        boot_bridge.framebuffer_data = RawData { start, size };
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

    pub fn font_data(&mut self, start: u64, size: usize) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.font_data = RawData { start, size };
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
        boot_bridge.mem_capacity = (boot_bridge
            .memory_map
            .entries()
            .filter(|e| {
                matches!(
                    e.ty,
                    MemoryType::CONVENTIONAL
                        | MemoryType::BOOT_SERVICES_CODE
                        | MemoryType::BOOT_SERVICES_DATA
                )
            })
            .map(|e| e.phys_start + (e.page_count * 4096))
            .max()
            .expect("Failed to get max memory")
            >> 30)
            + 1;
        self
    }

    pub fn rsdp(&mut self, rsdp: u64) -> &mut Self {
        let boot_bridge = self.inner_bridge();
        boot_bridge.rsdp = rsdp;
        self
    }

    /// Build the boot bridge and const return a pointer to it
    /// Failed if the boot bridge is not initialized
    pub fn build(self) -> Option<*const RawBootBridge> {
        self.boot_bridge
            .get()
            .copied()
            .map(|e| e as *const RawBootBridge)
    }
}

impl RawData {
    pub fn start(&self) -> u64 {
        self.start
    }
    pub fn size(&self) -> usize {
        self.size
    }
    pub fn end(&self) -> u64 {
        self.start + self.size as u64 - 1
    }
}

impl<'a> MemoryMap<'a> {
    pub fn new(memory_map: &'static [u8], entry_size: usize) -> Self {
        MemoryMap {
            memory_map,
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

impl MemoryDescriptor {
    pub fn phys_align(&self, align: u64) -> Option<Self> {
        if !align.is_power_of_two() {
            return None;
        }
        let mut aligned_self = *self;
        let ptr = self.phys_start as *const u8;
        aligned_self.phys_start += ptr.align_offset(align as usize) as u64;
        aligned_self.page_count =
            self.page_count - (aligned_self.phys_start - self.phys_start) / PAGE_SIZE - 1;
        if aligned_self.phys_start >= self.page_count * PAGE_SIZE + self.phys_start {
            return None;
        }
        return Some(aligned_self);
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
            "BootBridge {{ framebuffer_data: {:?}, font_data: {:?}, kernel_elf: {:?}, kernel_config: {:?}, rsdp: {}, mem_capacity: {} }}",
            boot_bridge.framebuffer_data,
            boot_bridge.font_data,
            boot_bridge.kernel_elf,
            boot_bridge.kernel_config,
            boot_bridge.rsdp,
            boot_bridge.mem_capacity
        )
    }
}
