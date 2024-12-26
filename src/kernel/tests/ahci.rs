#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![reexport_test_harness_main = "test_main"]
#![test_runner(radium::test_runner)]

extern crate alloc;
extern crate radium;

use core::usize;

use alloc::vec;
use common::boot::BootInformation;
use radium::{
    driver::storage::{ahci_driver::get_ahci, Drive},
    task::{executor::Executor, AwaitType, Task},
};
use x86_64::instructions::random;

#[no_mangle]
pub extern "C" fn start(boot_info_address: *mut BootInformation) -> ! {
    radium::init(boot_info_address);
    test_main();
    loop {}
}

const TEST_SIZE_IN_SECTOR: usize = 256; // 512 per sector
const SECTOR_TEST_RANGE: u64 = 256;
#[test_case]
fn simple_read_write() {
    let mut executor = Executor::new();
    executor.spawn(Task::new(
        async {
            let mut controller = get_ahci().get_contoller().lock();
            let mut backup_data = vec![0u8; TEST_SIZE_IN_SECTOR * 512];
            let mut data = vec![0u8; TEST_SIZE_IN_SECTOR * 512];
            get_random(&mut data);

            let drive = controller.get_drive(0).expect("Cannot get drive");
            drive
                .read(0, &mut backup_data, TEST_SIZE_IN_SECTOR)
                .await
                .unwrap();
            drive.write(0, &data, TEST_SIZE_IN_SECTOR).await.unwrap();

            let mut read_data = [0u8; TEST_SIZE_IN_SECTOR * 512];
            drive
                .read(0, &mut read_data, TEST_SIZE_IN_SECTOR)
                .await
                .unwrap();
            for (read, wrote) in data.iter().zip(read_data) {
                assert_eq!(*read, wrote);
            }
            drive
                .write(0, &backup_data, TEST_SIZE_IN_SECTOR)
                .await
                .unwrap();
        },
        AwaitType::Poll,
    ));

    executor.run_exit();
}

fn get_random(buffer: &mut [u8]) {
    let mut random_data = [0u16; TEST_SIZE_IN_SECTOR * 256];
    let rdrand = random::RdRand::new();
    for data in random_data.iter_mut() {
        if let Some(rdrand) = rdrand {
            *data = rdrand.get_u16().unwrap_or(16);
        } else {
            *data = 1;
        }
    }

    for (i, &num) in random_data.iter().enumerate() {
        let index = i * 2;
        buffer[index] = (num >> 8) as u8;
        buffer[index + 1] = num as u8;
    }
}

#[test_case]
fn sector_read_write() {
    let mut executor = Executor::new();
    executor.spawn(Task::new(
        async {
            let mut controller = get_ahci().get_contoller().lock();
            let mut backup_data = vec![0u8; TEST_SIZE_IN_SECTOR * 512];
            let mut data = vec![0u8; TEST_SIZE_IN_SECTOR * 512];

            let drive = controller.get_drive(0).expect("Cannot get drive");
            for sector in 0..SECTOR_TEST_RANGE {
                get_random(&mut data);
                drive
                    .read(sector, &mut backup_data, TEST_SIZE_IN_SECTOR)
                    .await
                    .unwrap();
                drive
                    .write(sector, &data, TEST_SIZE_IN_SECTOR)
                    .await
                    .unwrap();

                let mut read_data = [0u8; TEST_SIZE_IN_SECTOR * 512];
                drive
                    .read(sector, &mut read_data, TEST_SIZE_IN_SECTOR)
                    .await
                    .unwrap();
                for (read, wrote) in data.iter().zip(read_data) {
                    assert_eq!(*read, wrote);
                }
                drive
                    .write(sector, &backup_data, TEST_SIZE_IN_SECTOR)
                    .await
                    .unwrap();
            }
        },
        AwaitType::Poll,
    ));

    executor.run_exit();
}
