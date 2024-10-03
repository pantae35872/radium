use core::slice;

use elf_rs::Elf;
use uefi::{
    proto::console::gop::ModeInfo,
    table::boot::{MemoryMap, MemoryType, PAGE_SIZE},
};

#[repr(C)]
#[derive(Debug)]
pub struct BootInformation {
    // Max memory (in GiB) need to be all identity mapped for the kernel to be able to boot
    max_memory: u64,
    gop_mode_info: ModeInfo,
    framebuffer: *mut u32, /* &'static mut [u32]*/
    framebuffer_len: usize,
    runtime_system_table: u64,
    memory_map: MemoryMap<'static>,
    kernel_start: u64,
    kernel_size: usize,
    elf_section: Elf<'static>,
    font_start: u64,
    font_size: usize,
}

impl BootInformation {
    pub unsafe fn from_ptr_mut(bootinfo: *mut BootInformation) -> &'static mut Self {
        return &mut *(bootinfo);
    }

    pub unsafe fn from_ptr(bootinfo: *const BootInformation) -> &'static Self {
        return &*(bootinfo);
    }

    pub fn init_memory(&mut self, memory_map: MemoryMap<'static>, runtime_system_table: u64) {
        self.memory_map = memory_map;
        self.max_memory = self
            .memory_map
            .entries()
            .filter(|e| e.ty == MemoryType::CONVENTIONAL)
            .map(|e| e.page_count * PAGE_SIZE as u64)
            .sum::<u64>()
            .next_power_of_two()
            >> 30;
        self.runtime_system_table = runtime_system_table;
    }

    pub fn init_kernel(
        &mut self,
        font_start: u64,
        font_size: usize,
        kernel_start: u64,
        kernel_size: usize,
        elf: Elf<'static>,
    ) {
        self.kernel_start = kernel_start;
        self.kernel_size = kernel_size;
        self.elf_section = elf;
        self.font_start = font_start;
        self.font_size = font_size;
    }

    pub fn init_graphics(
        &mut self,
        mode_info: ModeInfo,
        framebuffer_start: u64,
        framebuffer_size: usize,
    ) {
        self.gop_mode_info = mode_info;
        self.framebuffer = framebuffer_start as *mut u32;
        self.framebuffer_len = framebuffer_size * size_of::<u32>();
    }

    pub fn max_memory(&self) -> u64 {
        return self.max_memory;
    }

    pub fn gop_mode_info(&self) -> &ModeInfo {
        return &self.gop_mode_info;
    }

    pub unsafe fn framebuffer(&self) -> &'static mut [u32] {
        slice::from_raw_parts_mut(self.framebuffer, self.framebuffer_len)
    }

    pub fn framebuffer_addr(&self) -> u64 {
        self.framebuffer as u64
    }

    pub fn framebuffer_size(&self) -> usize {
        self.framebuffer_len / size_of::<u32>()
    }

    pub fn runtime_system_table(&self) -> u64 {
        self.runtime_system_table
    }

    pub fn memory_map(&self) -> &MemoryMap<'static> {
        &self.memory_map
    }

    pub fn elf_section(&self) -> &Elf<'static> {
        &self.elf_section
    }

    pub fn font_addr(&self) -> u64 {
        self.font_start
    }

    pub fn font_size(&self) -> usize {
        self.font_size
    }

    pub unsafe fn font(&self) -> &'static mut [u8] {
        slice::from_raw_parts_mut(self.font_start as *mut u8, self.font_size)
    }
}
