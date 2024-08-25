use common::toml::{self, parser::TomlValue};
use uefi::{
    proto::{
        loaded_image::LoadedImage,
        media::{
            file::{File, FileInfo, FileMode, RegularFile},
            fs::SimpleFileSystem,
        },
    },
    table::{boot::MemoryType, Boot, SystemTable},
    CStr16,
};
use uefi_raw::protocol::file_system::FileAttribute;

pub fn read_config(system_table: &mut SystemTable<Boot>, config_path: &str) -> TomlValue {
    let file = read_file(system_table, config_path);
    let info_file = core::str::from_utf8(file).expect("Info file is not valid utf8");

    return toml::parse_toml(info_file).expect("Cannot parse kernel info file");
}

pub fn read_file(system_table: &mut SystemTable<Boot>, path: &str) -> &'static [u8] {
    let protocol = system_table
        .boot_services()
        .open_protocol_exclusive::<LoadedImage>(system_table.boot_services().image_handle())
        .expect("Failed to open protocol for loaded image");

    let loaded_image_protocol = protocol.get().expect("Failed to get loaded image protocol");

    let device_handle = loaded_image_protocol
        .device()
        .expect("Failed to get device handle");

    let simple_fs_protocol = system_table
        .boot_services()
        .open_protocol_exclusive::<SimpleFileSystem>(device_handle)
        .expect("Failed to open simple file system protocol");

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
        match root_directory.open(&filename, FileMode::Read, FileAttribute::READ_ONLY) {
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
    return buffer;
}
