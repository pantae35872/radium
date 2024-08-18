#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![reexport_test_harness_main = "test_main"]
#![test_runner(nothingos::test_runner)]

extern crate nothingos;

use core::usize;

use common::BootInformation;
use nothingos::driver::storage::{ahci_driver, Drive};
use x86_64::instructions::random;

#[no_mangle]
pub extern "C" fn start(multiboot_information_address: *mut BootInformation) -> ! {
    nothingos::init(multiboot_information_address);
    test_main();
    loop {}
}

const TEST_SIZE_IN_SECTOR: usize = 32; // 512 per sector
const SECTOR_TEST_RANGE: u64 = 256;
#[test_case]
fn simple_read_write() {
    let mut controller = ahci_driver::DRIVER
        .get()
        .expect("AHCI Driver is not initialize")
        .lock();
    let mut backup_data = [0u8; TEST_SIZE_IN_SECTOR * 512];
    let mut data = [0u8; TEST_SIZE_IN_SECTOR * 512];
    get_random(&mut data);

    let drive = controller.get_drive(0).expect("Cannot get drive");
    drive
        .read(0, &mut backup_data, TEST_SIZE_IN_SECTOR)
        .unwrap();
    drive.write(0, &data, TEST_SIZE_IN_SECTOR).unwrap();

    let mut read_data = [0u8; TEST_SIZE_IN_SECTOR * 512];
    drive.read(0, &mut read_data, TEST_SIZE_IN_SECTOR).unwrap();
    for (read, wrote) in data.iter().zip(read_data) {
        assert_eq!(*read, wrote);
    }
    drive.write(0, &backup_data, TEST_SIZE_IN_SECTOR).unwrap();
}

fn get_random(buffer: &mut [u8]) {
    let mut random_data = [0u16; TEST_SIZE_IN_SECTOR * 256];
    let rdrand = random::RdRand::new().unwrap();
    for data in random_data.iter_mut() {
        *data = rdrand.get_u16().expect("Cannot get random");
    }

    for (i, &num) in random_data.iter().enumerate() {
        let index = i * 2;
        buffer[index] = (num >> 8) as u8;
        buffer[index + 1] = num as u8;
    }
}

#[test_case]
fn sector_read_write() {
    let mut controller = ahci_driver::DRIVER
        .get()
        .expect("AHCI Driver is not initialize")
        .lock();
    let mut backup_data = [0u8; TEST_SIZE_IN_SECTOR * 512];
    let mut data = [0u8; TEST_SIZE_IN_SECTOR * 512];

    let drive = controller.get_drive(0).expect("Cannot get drive");
    for sector in 0..SECTOR_TEST_RANGE {
        get_random(&mut data);
        drive
            .read(sector, &mut backup_data, TEST_SIZE_IN_SECTOR)
            .unwrap();
        drive.write(sector, &data, TEST_SIZE_IN_SECTOR).unwrap();

        let mut read_data = [0u8; TEST_SIZE_IN_SECTOR * 512];
        drive
            .read(sector, &mut read_data, TEST_SIZE_IN_SECTOR)
            .unwrap();
        for (read, wrote) in data.iter().zip(read_data) {
            assert_eq!(*read, wrote);
        }
        drive
            .write(sector, &backup_data, TEST_SIZE_IN_SECTOR)
            .unwrap();
    }
}
