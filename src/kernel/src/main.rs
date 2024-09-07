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
use nothingos::memory::allocator::buddy_allocator::BuddyAllocator;
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

    let mut buf = [0u8; 256];
    let mut heap = unsafe { BuddyAllocator::<8>::new(buf.as_mut_ptr() as usize, buf.len()) };

    println!("{:?}, {:?}", heap.allocate(8), buf.as_ptr());
    println!("{:?}, {:?}", heap.allocate(8), buf.as_ptr());
    println!("{:?}, {:?}", heap.allocate(8), buf.as_ptr());
    println!("{:?}, {:?}", heap.allocate(16), buf.as_ptr());
    /*instructions::interrupts::without_interrupts(|| {
        SCHEDULER
            .get()
            .unwrap()
            .lock()
            .add_process(Process::new(10, "1".into()))
            .add_process(Process::new(2, "2".into()))
            .add_process(Process::new(10, "3".into()))
            .add_process(Process::new(2, "4".into()))
            .add_process(Process::new(10, "5".into()))
            .add_process(Process::new(2, "6".into()));
    });*/
    /*gpt.format().unwrap();
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
    .expect("Error");*/
    //let partition1 = gpt.read_partition(1).expect("Error");
    //println!("{}", partition1.get_partition_name());
    let mut executor = Executor::new();
    executor.spawn(Task::new(
        async {
            let mut controller = get_ahci().get_contoller().lock();
            let drive = controller.get_drive(0).expect("Cannot get drive");
            let mut gpt = GPTPartitions::new(drive.into());
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
