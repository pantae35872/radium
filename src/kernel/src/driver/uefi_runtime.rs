// This code is mostly taken from uefi-rs library, because the uefi-rs library define it's own
// Panic handler which causes some conflicts with the kernel panic handler so i'll just copy just
// the runtime part of the uefi-rs library
//
// Copyright (c) The uefi-rs contributors
//
// Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the
// Software without restriction, including without
// limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software
// is furnished to do so, subject to the following
// conditions:
//
// The above copyright notice and this permission notice
// shall be included in all copies or substantial portions
// of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
// ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
// TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
// PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
// SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
// OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
// IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use core::{ffi::c_void, mem::MaybeUninit};

use bootbridge::{MemoryDescriptor, MemoryMap, MemoryType};
use c_enum::c_enum;
use conquer_once::spin::OnceCell;
use pager::{
    address::{PhysAddr, VirtAddr},
    Mapper, PAGE_SIZE,
};
use sentinel::log;
use spin::Mutex;
use uguid::Guid;

use crate::{
    initialization_context::{FinalPhase, InitializationContext},
    memory::virt_addr_alloc,
};

#[repr(C)]
struct SystemTable {
    header: TableHeader,

    firmware_vendor: *const c_void,
    firmware_revision: u32,

    stdin_handle: *mut c_void,
    stdin: *mut c_void,

    stdout_handle: *mut c_void,
    stdout: *mut c_void,

    stderr_handle: *mut c_void,
    stderr: *mut c_void,

    runtime_services: *mut RuntimeServices, // We only care about Runtime service
    boot_services: *mut c_void,

    number_of_configuration_table_entries: usize,
    configuration_table: *mut c_void,
}

#[repr(C)]
struct TableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    _reserved: u32,
}

c_enum! {
    pub enum EfiStatus: u64 {
        SUCCESS = 0

        WARN_UNKNOWN_GLYPH      =  1
        WARN_DELETE_FAILURE     =  2
        WARN_WRITE_FAILURE      =  3
        WARN_BUFFER_TOO_SMALL   =  4
        WARN_STALE_DATA         =  5
        WARN_FILE_SYSTEM        =  6
        WARN_RESET_REQUIRED     =  7

        LOAD_ERROR              = Self::ERROR_BIT |  1
        INVALID_PARAMETER       = Self::ERROR_BIT |  2
        UNSUPPORTED             = Self::ERROR_BIT |  3
        BAD_BUFFER_SIZE         = Self::ERROR_BIT |  4
        BUFFER_TOO_SMALL        = Self::ERROR_BIT |  5
        NOT_READY               = Self::ERROR_BIT |  6
        DEVICE_ERROR            = Self::ERROR_BIT |  7
        WRITE_PROTECTED         = Self::ERROR_BIT |  8
        OUT_OF_RESOURCES        = Self::ERROR_BIT |  9
        VOLUME_CORRUPTED        = Self::ERROR_BIT | 10
        VOLUME_FULL             = Self::ERROR_BIT | 11
        NO_MEDIA                = Self::ERROR_BIT | 12
        MEDIA_CHANGED           = Self::ERROR_BIT | 13
        NOT_FOUND               = Self::ERROR_BIT | 14
        ACCESS_DENIED           = Self::ERROR_BIT | 15
        NO_RESPONSE             = Self::ERROR_BIT | 16
        NO_MAPPING              = Self::ERROR_BIT | 17
        TIMEOUT                 = Self::ERROR_BIT | 18
        NOT_STARTED             = Self::ERROR_BIT | 19
        ALREADY_STARTED         = Self::ERROR_BIT | 20
        ABORTED                 = Self::ERROR_BIT | 21
        ICMP_ERROR              = Self::ERROR_BIT | 22
        TFTP_ERROR              = Self::ERROR_BIT | 23
        PROTOCOL_ERROR          = Self::ERROR_BIT | 24
        INCOMPATIBLE_VERSION    = Self::ERROR_BIT | 25
        SECURITY_VIOLATION      = Self::ERROR_BIT | 26
        CRC_ERROR               = Self::ERROR_BIT | 27
        END_OF_MEDIA            = Self::ERROR_BIT | 28
        END_OF_FILE             = Self::ERROR_BIT | 31
        INVALID_LANGUAGE        = Self::ERROR_BIT | 32
        COMPROMISED_DATA        = Self::ERROR_BIT | 33
        IP_ADDRESS_CONFLICT     = Self::ERROR_BIT | 34
        HTTP_ERROR              = Self::ERROR_BIT | 35
    }

    pub enum ResetType: u32 {
        COLD = 0
        WARM = 1
        SHUTDOWN = 2
        PLATFORM_SPECIFIC = 3
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct VariableAttributes: u32 {
        /// Variable is maintained across a power cycle.
        const NON_VOLATILE = 0x01;

        /// Variable is accessible during the time that boot services are
        /// accessible.
        const BOOTSERVICE_ACCESS = 0x02;

        /// Variable is accessible during the time that runtime services are
        /// accessible.
        const RUNTIME_ACCESS = 0x04;

        /// Variable is stored in the portion of NVR allocated for error
        /// records.
        const HARDWARE_ERROR_RECORD = 0x08;

        /// Deprecated.
        const AUTHENTICATED_WRITE_ACCESS = 0x10;

        /// Variable payload begins with an EFI_VARIABLE_AUTHENTICATION_2
        /// structure.
        const TIME_BASED_AUTHENTICATED_WRITE_ACCESS = 0x20;

        /// This is never set in the attributes returned by
        /// `get_variable`. When passed to `set_variable`, the variable payload
        /// will be appended to the current value of the variable if supported
        /// by the firmware.
        const APPEND_WRITE = 0x40;

        /// Variable payload begins with an EFI_VARIABLE_AUTHENTICATION_3
        /// structure.
        const ENHANCED_AUTHENTICATED_ACCESS = 0x80;
    }
}

impl EfiStatus {
    pub const ERROR_BIT: u64 = 1 << (core::mem::size_of::<u64>() * 8 - 1);

    #[inline]
    #[must_use]
    pub fn is_success(self) -> bool {
        self == EfiStatus::SUCCESS
    }

    #[inline]
    #[must_use]
    pub fn is_warning(self) -> bool {
        (self != EfiStatus::SUCCESS) && (self.0 & Self::ERROR_BIT == 0)
    }

    #[inline]
    #[must_use]
    pub const fn is_error(self) -> bool {
        self.0 & Self::ERROR_BIT != 0
    }
}

/// Date and time representation.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq)]
#[repr(C)]
pub struct Time {
    /// Year. Valid range: `1900..=9999`.
    pub year: u16,

    /// Month. Valid range: `1..=12`.
    pub month: u8,

    /// Day of the month. Valid range: `1..=31`.
    pub day: u8,

    /// Hour. Valid range: `0..=23`.
    pub hour: u8,

    /// Minute. Valid range: `0..=59`.
    pub minute: u8,

    /// Second. Valid range: `0..=59`.
    pub second: u8,

    /// Unused padding.
    pub pad1: u8,

    /// Nanosececond. Valid range: `0..=999_999_999`.
    pub nanosecond: u32,

    /// Offset in minutes from UTC. Valid range: `-1440..=1440`, or
    /// [`Time::UNSPECIFIED_TIMEZONE`].
    pub time_zone: i16,

    /// Daylight savings time information.
    pub daylight: Daylight,

    /// Unused padding.
    pub pad2: u8,
}

bitflags! {
    /// A bitmask containing daylight savings time information.
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Daylight: u8 {
        /// Time is affected by daylight savings time.
        const ADJUST_DAYLIGHT = 0x01;

        /// Time has been adjusted for daylight savings time.
        const IN_DAYLIGHT = 0x02;
    }
}

#[repr(C)]
struct RuntimeServices {
    header: TableHeader,
    get_time: unsafe extern "efiapi" fn(time: *mut Time, capabilities: *mut c_void) -> EfiStatus,
    set_time: unsafe extern "efiapi" fn(time: *const Time) -> EfiStatus,
    get_wakeup_time:
        unsafe extern "efiapi" fn(enabled: *mut u8, pending: *mut u8, time: *mut Time) -> EfiStatus,
    set_wakeup_time: unsafe extern "efiapi" fn(enable: u8, time: *const Time) -> EfiStatus,
    set_virtual_address_map: unsafe extern "efiapi" fn(
        map_size: usize,
        desc_size: usize,
        desc_version: u32,
        virtual_map: *mut MemoryDescriptor,
    ) -> EfiStatus,
    convert_pointer: unsafe extern "efiapi" fn(
        debug_disposition: usize,
        address: *mut *const c_void,
    ) -> EfiStatus,
    get_variable: unsafe extern "efiapi" fn(
        variable_name: *const u16,
        vendor_guid: *const Guid,
        attributes: *mut VariableAttributes,
        data_size: *mut usize,
        data: *mut u8,
    ) -> EfiStatus,
    get_next_variable_name: unsafe extern "efiapi" fn(
        variable_name_size: *mut usize,
        variable_name: *mut u16,
        vendor_guid: *mut Guid,
    ) -> EfiStatus,
    set_variable: unsafe extern "efiapi" fn(
        variable_name: *const u16,
        vendor_guid: *const Guid,
        attributes: VariableAttributes,
        data_size: usize,
        data: *const u8,
    ) -> EfiStatus,
    get_next_high_monotonic_count: unsafe extern "efiapi" fn(high_count: *mut u32) -> EfiStatus,
    reset_system: unsafe extern "efiapi" fn(
        rt: ResetType,
        status: EfiStatus,
        data_size: usize,
        data: *const u8,
    ) -> !,

    // UEFI 2.0 Capsule Services.
    update_capsule: unsafe extern "efiapi" fn(
        capsule_header_array: *const *const c_void,
        capsule_count: usize,
        scatter_gather_list: PhysAddr,
    ) -> EfiStatus,
    query_capsule_capabilities: unsafe extern "efiapi" fn(
        capsule_header_array: *const *const c_void,
        capsule_count: usize,
        maximum_capsule_size: *mut usize,
        reset_type: *mut ResetType,
    ) -> EfiStatus,

    // Miscellaneous UEFI 2.0 Service.
    query_variable_info: unsafe extern "efiapi" fn(
        attributes: VariableAttributes,
        maximum_variable_storage_size: *mut u64,
        remaining_variable_storage_size: *mut u64,
        maximum_variable_size: *mut u64,
    ) -> EfiStatus,
}

pub static UEFI_RUNTIME: OnceCell<Mutex<UefiRuntime>> = OnceCell::uninit();

pub struct UefiRuntime {
    table: &'static mut SystemTable,
}

unsafe impl Send for UefiRuntime {}
unsafe impl Sync for UefiRuntime {}

impl UefiRuntime {
    fn new(ctx: &mut InitializationContext<FinalPhase>) -> Self {
        let mut mem_map: MemoryMap<'static> =
            ctx.context().boot_bridge().memory_map().clone().into();
        let runtime_table_raw = ctx.context().boot_bridge().uefi_runtime_ptr();

        log!(
            Info,
            "Initializing UEFI Runtime, RUNTIME TABLE PHYS ADDR AT: {:#x}",
            runtime_table_raw.as_u64()
        );

        let mut runtime_table = VirtAddr::null();
        for ufu_stuff in mem_map.entries_mut().filter(|e| {
            matches!(
                e.ty,
                MemoryType::RUNTIME_SERVICES_CODE | MemoryType::RUNTIME_SERVICES_DATA
            )
        }) {
            let page = virt_addr_alloc(ufu_stuff.page_count);
            unsafe {
                ctx.mapper().map_to_range_by_size(
                    page,
                    ufu_stuff.phys_start.into(),
                    (ufu_stuff.page_count * PAGE_SIZE) as usize,
                    ufu_stuff.att.into(),
                );
                ctx.mapper().identity_map_by_size(
                    ufu_stuff.phys_start.into(),
                    (ufu_stuff.page_count * PAGE_SIZE) as usize,
                    ufu_stuff.att.into(),
                );
            };
            ufu_stuff.virt_start = page.start_address();
            if ufu_stuff.phys_start < runtime_table_raw
                && (ufu_stuff.phys_start + ufu_stuff.phys_start.as_u64() * PAGE_SIZE - 1)
                    > runtime_table_raw
            {
                runtime_table = page.start_address()
                    + (runtime_table_raw.as_u64() - ufu_stuff.phys_start.as_u64());
            }
        }

        assert!(
            !runtime_table.is_null(),
            "Runtime table is not in the uefi memory range"
        );

        log!(
            Debug,
            "Mapped UEFI Runtime, UEFI Runtime table vaddr at: {:#x}",
            runtime_table
        );
        let runtime_table = unsafe { &mut *runtime_table.as_mut_ptr::<SystemTable>() };
        unsafe {
            let status = ((*runtime_table.runtime_services).set_virtual_address_map)(
                mem_map.size(),
                mem_map.entry_size(),
                mem_map.entry_version() as u32,
                mem_map.as_ptr().cast_mut().cast(),
            );
            log!(Debug, "EFI SET VIRTUAL ADDRESS MAP STATUS: {status:?}");
            if status != EfiStatus::SUCCESS {
                panic!("Failed to set virtual address map for uefi {status:?}, DescSize: {}, DescVersion: {}", mem_map.entry_size(), mem_map.entry_version());
            }
        };

        for ufu_stuff in mem_map.entries_mut().filter(|e| {
            matches!(
                e.ty,
                MemoryType::RUNTIME_SERVICES_CODE | MemoryType::RUNTIME_SERVICES_DATA
            )
        }) {
            unsafe {
                ctx.mapper().unmap_addr_by_size(
                    VirtAddr::new(ufu_stuff.phys_start.as_u64()).into(),
                    (ufu_stuff.page_count * PAGE_SIZE) as usize,
                )
            }
        }

        Self {
            table: runtime_table,
        }
    }

    pub fn get_time(&self) -> Time {
        let mut time = MaybeUninit::<Time>::uninit();
        let status = unsafe {
            ((*self.table.runtime_services).get_time)(
                time.as_mut_ptr().cast(),
                core::ptr::null_mut(),
            )
        };
        if status != EfiStatus::SUCCESS {
            panic!("Failed to set virtual address map for uefi {status:?}");
        }
        unsafe { time.assume_init() }
    }

    pub fn reset(&self, rt: ResetType, status: EfiStatus) -> ! {
        unsafe { ((*self.table.runtime_services).reset_system)(rt, status, 0, core::ptr::null()) }
    }
}

pub fn uefi_runtime() -> &'static Mutex<UefiRuntime> {
    UEFI_RUNTIME.get().expect("UEFI Runtime not initialized")
}

pub fn init(ctx: &mut InitializationContext<FinalPhase>) {
    UEFI_RUNTIME.init_once(|| UefiRuntime::new(ctx).into());
}
