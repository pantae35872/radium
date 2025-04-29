use alloc::ffi::CString;
use boot_cfg_parser::toml::{parse_toml, parser::TomlValue};
use uefi::{
    proto::{
        loaded_image::LoadedImage,
        media::{
            file::{File, FileInfo, FileMode, RegularFile},
            fs::SimpleFileSystem,
        },
        network::{pxe::BaseCode, IpAddress},
    },
    table::boot::MemoryType,
    CStr16,
};

use uefi_raw::protocol::file_system::FileAttribute;
use uefi_services::{println, system_table};

const DEV_SERVER_IP: IpAddress = IpAddress::new_v4([192, 168, 69, 1]);

/// A read only file, just an abstraction over the uefi file protocol
pub struct LoaderFile {
    buffer: *const [u8],
    mark_permanent: bool,
}

impl LoaderFile {
    pub fn new(path: &str) -> Self {
        let system_table = system_table();
        let protocol = system_table
            .boot_services()
            .open_protocol_exclusive::<LoadedImage>(system_table.boot_services().image_handle())
            .expect("Failed to open protocol for loaded image");

        let loaded_image_protocol = protocol.get().expect("Failed to get loaded image protocol");

        let device_handle = loaded_image_protocol
            .device()
            .expect("Failed to get device handle");

        let simple_fs_protocol = match system_table
            .boot_services()
            .open_protocol_exclusive::<SimpleFileSystem>(device_handle)
        {
            Ok(simple_fs_protocol) => simple_fs_protocol,
            Err(_) => {
                println!("No simple file system found, assuming testing with PXE");
                let mut base_code = system_table
                    .boot_services()
                    .open_protocol_exclusive::<BaseCode>(device_handle)
                    .unwrap();

                let cstring = CString::new(path).unwrap();
                let cstr = cstring.as_c_str().try_into().unwrap();

                let buffer = base_code
                    .tftp_get_file_size(&DEV_SERVER_IP, cstr)
                    .expect("Failed to get file size for PXE Boot");
                let buffer: &mut [u8] = unsafe {
                    core::slice::from_raw_parts_mut(
                        system_table
                            .boot_services()
                            .allocate_pool(MemoryType::LOADER_DATA, buffer as usize)
                            .unwrap(),
                        buffer as usize,
                    )
                };
                base_code
                    .tftp_read_file(&DEV_SERVER_IP, cstr, Some(buffer))
                    .expect("Directory not found");

                return Self {
                    buffer: buffer as *const [u8],
                    mark_permanent: false,
                };
            }
        };

        let simple_file_system = match simple_fs_protocol.get_mut() {
            Some(protocol) => protocol,
            None => panic!("Could not get protocol from scoped protocol (Simple File System)"),
        };

        let mut root_directory = match simple_file_system.open_volume() {
            Ok(dir) => dir,
            Err(error) => panic!(
                "Could not open volume from simple file system protocol {}",
                error
            ),
        };
        let mut buf = [0; 64];
        let filename = match CStr16::from_str_with_buf(path, &mut buf) {
            Ok(filename) => filename,
            Err(error) => panic!("could not create a file name for kernel info {}", error),
        };

        let mut file: RegularFile =
            match root_directory.open(filename, FileMode::Read, FileAttribute::READ_ONLY) {
                Ok(file) => match file.into_regular_file() {
                    Some(file) => file,
                    None => panic!("A info file for an kernel is not a file"),
                },
                Err(error) => panic!("Could not open info file for the kernel {}", error),
            };

        let mut buffer = [0u8; 512];
        let info: &mut FileInfo = match file.get_info(&mut buffer) {
            Ok(value) => value,
            Err(panic) => panic!("Could not get info for a kernel info file: {}", panic),
        };

        let buffer: &mut [u8] = unsafe {
            core::slice::from_raw_parts_mut(
                system_table
                    .boot_services()
                    .allocate_pool(MemoryType::LOADER_DATA, info.file_size() as usize)
                    .unwrap(),
                info.file_size() as usize,
            )
        };
        file.read(buffer).expect("Cannot read file");
        Self {
            buffer: buffer as *const [u8],
            mark_permanent: false,
        }
    }

    /// Borrow a buffer of a file
    pub fn buffer(&self) -> &[u8] {
        unsafe { &*self.buffer }
    }

    /// Consume self, create a permanent buffer of a file
    pub fn permanent(mut self) -> &'static [u8] {
        self.mark_permanent = true;
        unsafe { &*self.buffer }
    }
}

impl Drop for LoaderFile {
    fn drop(&mut self) {
        if self.mark_permanent {
            return;
        }
        unsafe {
            system_table()
                .boot_services()
                .free_pool(self.buffer as *mut u8)
                .expect("Failed to deallocate an unused file");
        }
    }
}

impl From<LoaderFile> for TomlValue {
    fn from(value: LoaderFile) -> Self {
        let buffer = value.buffer();
        parse_toml(
            core::str::from_utf8(buffer)
                .expect("File is not a valid utf8, can't convert into toml value"),
        )
        .expect("Failed to parse a toml file")
    }
}
