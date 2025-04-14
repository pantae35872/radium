#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate radium;
extern crate spin;

use bootbridge::RawBootBridge;
use radium::logger::LOGGER;
use radium::task::executor::Executor;
use radium::task::{AwaitType, Task};
use radium::{print, println, serial_print};

// TODO: Implements acpi to get io apic
// TODO: Use ahci interrupt (needs io apic) with waker
// TODO: Implements waker based async mutex
// TODO: Impelemnts kernel services executor

#[no_mangle]
pub extern "C" fn start(boot_bridge: *const RawBootBridge) -> ! {
    radium::init(boot_bridge);
    println!("Hello, world!!");
    #[cfg(not(feature = "testing"))]
    LOGGER.flush_all(&[/*|s| print!("{s}"),*/ |s| serial_print!("{s}")]);
    //println!("Time Test: {:?}", uefi_runtime().get_time());
    let mut executor = Executor::new();
    executor.spawn(Task::new(
        async {
            //let mut controller = get_ahci().get_contoller().lock();
            //let drive = controller.get_drive(0).expect("Cannot get drive");
            //let mut gpt = GPTPartitions::new(drive);

            //gpt.format().await.unwrap();
            //gpt.set_partiton(
            //    1,
            //    &guid!("0FC63DAF-8483-4772-8E79-3D69D8477DE4"),
            //    34,
            //    2048,
            //    0,
            //    &{
            //        let mut array = [0; 72];
            //        let string: Vec<u8> = "My partition"
            //            .encode_utf16()
            //            .flat_map(|c| vec![(c & 0xFF) as u8, (c >> 8) as u8])
            //            .collect();
            //        array[..string.len()].copy_from_slice(string.as_slice());
            //        array
            //    },
            //)
            //.await
            //.expect("Error");
            //let partition1 = gpt.read_partition(1).await.expect("Error");
            //log!(Debug, "{}", partition1.get_partition_name());
        },
        AwaitType::Poll,
    ));

    #[cfg(test)]
    test_main();

    executor.run();
}
