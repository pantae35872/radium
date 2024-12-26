use core::{
    ffi::c_void,
    slice,
    sync::atomic::{AtomicPtr, Ordering},
};

use elf_rs::Elf;
use uefi::{
    proto::console::gop::ModeInfo,
    table::{
        boot::{MemoryMap, MemoryType, PAGE_SIZE},
        Runtime, SystemTable,
    },
};

#[repr(C)]
#[derive(Debug)]
pub struct BootInformation {
    // Max memory (in GiB) need to be all identity mapped for the kernel to be able to boot
    max_memory: u64,
    gop_mode_info: ModeInfo,
    framebuffer: AtomicPtr<u32>, /* &'static mut [u32]*/
    framebuffer_len: usize,
    runtime_system_table: AtomicPtr<c_void>,
    memory_map: MemoryMap<'static>,
    kernel_start: u64,
    kernel_size: usize,
    elf_section: Elf<'static>,
    font_start: AtomicPtr<u8>,
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
        self.max_memory = (self
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
            .map(|e| e.phys_start + (e.page_count * PAGE_SIZE as u64))
            .max()
            .expect("Cannot get max mem")
            >> 30)
            + 1;
        self.runtime_system_table = AtomicPtr::new(runtime_system_table as *mut c_void);
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
        assert!(font_start != 0, "Font can't be null");
        self.font_start
            .store(font_start as *mut u8, Ordering::Relaxed);
        self.font_size = font_size;
    }

    pub fn init_graphics(
        &mut self,
        mode_info: ModeInfo,
        framebuffer_start: u64,
        framebuffer_len: usize,
    ) {
        self.gop_mode_info = mode_info;
        assert!(framebuffer_start != 0, "Framebuffer can't be null");
        self.framebuffer
            .store(framebuffer_start as *mut u32, Ordering::Relaxed);
        self.framebuffer_len = framebuffer_len;
    }

    pub fn max_memory(&self) -> u64 {
        return self.max_memory;
    }

    pub fn gop_mode_info(&self) -> &ModeInfo {
        return &self.gop_mode_info;
    }

    pub fn framebuffer(&self) -> Option<&'static mut [u32]> {
        let ptr = self
            .framebuffer
            .swap(core::ptr::null_mut(), Ordering::Acquire);
        if !ptr.is_null() {
            unsafe { Some(slice::from_raw_parts_mut(ptr, self.framebuffer_len)) }
        } else {
            None
        }
    }

    pub fn framebuffer_addr(&self) -> Option<u64> {
        let ptr = self.framebuffer.load(Ordering::Relaxed);
        if !ptr.is_null() {
            Some(ptr as u64)
        } else {
            None
        }
    }

    /// Frame buffer size in BYTES!!!
    pub fn framebuffer_size(&self) -> usize {
        self.framebuffer_len * size_of::<u32>()
    }

    pub fn runtime_system_table(&self) -> Option<SystemTable<Runtime>> {
        let ptr = self
            .runtime_system_table
            .swap(core::ptr::null_mut(), Ordering::Acquire);
        unsafe { SystemTable::<Runtime>::from_ptr(ptr) }
    }

    pub fn memory_map(&self) -> &MemoryMap<'static> {
        &self.memory_map
    }

    pub fn elf_section(&self) -> &Elf<'static> {
        &self.elf_section
    }

    pub fn font_addr(&self) -> Option<u64> {
        let font_start = self.font_start.load(Ordering::Relaxed);
        if !font_start.is_null() {
            Some(font_start as u64)
        } else {
            None
        }
    }

    pub fn font_size(&self) -> usize {
        self.font_size
    }

    pub fn font(&self) -> Option<&'static mut [u8]> {
        let font_start = self
            .font_start
            .swap(core::ptr::null_mut(), Ordering::Acquire);
        if !font_start.is_null() {
            unsafe {
                Some(slice::from_raw_parts_mut(
                    font_start as *mut u8,
                    self.font_size,
                ))
            }
        } else {
            None
        }
    }
}
