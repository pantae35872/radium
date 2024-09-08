#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(custom_test_frameworks)]
#![test_runner(nothingos::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate nothingos;
extern crate spin;

use core::arch::asm;

use alloc::ffi::CString;
use common::boot::BootInformation;
use nothingos::driver::storage::ahci_driver::get_ahci;
use nothingos::filesystem::partition::gpt_partition::GPTPartitions;
use nothingos::println;
use nothingos::task::executor::Executor;
use nothingos::task::{AwaitType, Task};

#[no_mangle]
fn sys_print(value: &str) {
    let string = CString::new(value).unwrap();
    unsafe {
        asm!("int 0x80", in("rax") 1, in("rcx") string.into_raw());
    }
}

#[no_mangle]
pub extern "C" fn start(information_address: *mut BootInformation) -> ! {
    nothingos::init(information_address);
    println!("Hello world!");
    let mut executor = Executor::new();
    executor.spawn(Task::new(
        async {
            let mut controller = get_ahci().get_contoller().lock();
            let drive = controller.get_drive(0).expect("Cannot get drive");
            let mut gpt = GPTPartitions::new(drive.into());

            /*gpt.format().await.unwrap();
            gpt.set_partiton(
                1,
                &guid!("0FC63DAF-8483-4772-8E79-3D69D8477DE4"),
                34,
                2048,
                0,
                &{
                    let mut array = [0; 72];
                    let string: Vec<u8> = "My partition"
                        .encode_utf16()
                        .flat_map(|c| vec![(c & 0xFF) as u8, (c >> 8) as u8])
                        .collect();
                    array[..string.len()].copy_from_slice(string.as_slice());
                    array
                },
            )
            .await
            .expect("Error");*/
            let partition1 = gpt.read_partition(1).await.expect("Error");
            println!("{}", partition1.get_partition_name());
        },
        AwaitType::Poll,
    ));
    executor.spawn(Task::new(
        async {
            println!("Task 2");
        },
        AwaitType::Poll,
    ));

    #[cfg(test)]
    test_main();

    executor.run();
}
